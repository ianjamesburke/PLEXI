//! Attention Queue & Decision Overlay (`SPEC.md` R35, R37, R38, R39, R41).
//!
//! Replaces the legacy notification modal, toasts, and pop-over approval banners
//! with:
//! 1. One ordered list of bars: `Ask` (amber `◆`, oldest first), `Failed` (red `✕`),
//!    `Result` (`●`), `Done` (`✓`). Bars never answer inline.
//! 2. Lock rule: when the focused pane has an unresolved ask, that ask is pinned
//!    at item 0 with a lock mark (`locked = true`) under any arrivals until
//!    answered or focus leaves the pane.
//! 3. Collapsed calm pill in the path line (`◆N · total` + top bar preview).
//! 4. Decision Overlay: opens only on explicit user action (`Enter` on an ask bar,
//!    `Enter` on an asking atlas node, or `Cmd+Alt+B`). Never opens automatically
//!    on arrival. Shows `"What would you do here?"`, monospace command text, and
//!    2–4 options (starred recommendation when present). Sends at most one verdict.

use crate::app::app_trait::KeyDisposition;
use crate::app::input_router::PlexiInput;
use crate::app::PlexiApp;
use crate::broker::gate::{ApprovalChoice, NeedsYouKind, PermissionMonitor, ResolveSurface};
use crate::ui::card_block::{layout_card, paint_card, wrap_text, CardForm, CardSpec, CardState};
use crate::ui::style;
use egui::{
    Align2, CornerRadius, FontFamily, FontId, Key, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const OVERLAY_QUESTION: &str = "What would you do here?";
pub const ESC_BACKOUT_HINT: &str = "Cmd+Esc backs out";
pub const NO_ASKS_HINT: &str = "No open asks in queue";

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum QueueCategory {
    Ask = 0,
    Failed = 1,
    Result = 2,
    Done = 3,
    /// Host lifecycle / integrity notices. Always sorted below every ask and
    /// cleared once seen; never counted as asks.
    Notice = 4,
}

impl QueueCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Needs you",
            Self::Failed => "Failed",
            Self::Result => "Result waiting",
            Self::Done => "Done",
            Self::Notice => "Notice",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueueOption {
    pub key: String,
    pub label: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum QueueSource {
    BrokerPending {
        request_id: String,
    },
    NeedsYou {
        request_id: String,
        is_question: bool,
    },
    AgentState {
        pane_id: u64,
    },
    ChildResult {
        pane_id: u64,
    },
    Notification {
        notify_id: String,
    },
    Synthetic {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueBar {
    pub id: String,
    pub category: QueueCategory,
    pub pane_id: u64,
    pub pane_name: String,
    pub context_id: u64,
    pub context_path: String,
    pub summary: String,
    pub command_text: String,
    pub options: Vec<QueueOption>,
    pub created_at_ms: u64,
    pub locked: bool,
    pub source: QueueSource,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecisionVerdict {
    pub ask_id: String,
    pub pane_id: u64,
    pub option_idx: usize,
    pub option_key: String,
    pub option_label: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DecisionOverlayState {
    pub ask_id: String,
    pub pane_id: u64,
    pub pane_name: String,
    pub context_id: u64,
    pub context_path: String,
    pub question: String,
    pub command_text: String,
    pub options: Vec<QueueOption>,
    pub selected_idx: usize,
    pub opened_at: Instant,
    pub hint: Option<(String, Instant)>,
    pub stale_notice: Option<(String, Instant)>,
    pub source: QueueSource,
}

impl DecisionOverlayState {
    pub fn from_bar(bar: &QueueBar) -> Self {
        let options = if bar.options.is_empty() {
            default_ask_options()
        } else {
            bar.options.clone()
        };
        let selected_idx = options.iter().position(|o| o.recommended).unwrap_or(0);
        Self {
            ask_id: bar.id.clone(),
            pane_id: bar.pane_id,
            pane_name: bar.pane_name.clone(),
            context_id: bar.context_id,
            context_path: bar.context_path.clone(),
            question: OVERLAY_QUESTION.to_string(),
            command_text: if bar.command_text.is_empty() {
                bar.summary.clone()
            } else {
                bar.command_text.clone()
            },
            options,
            selected_idx,
            opened_at: Instant::now(),
            hint: None,
            stale_notice: None,
            source: bar.source.clone(),
        }
    }
}

pub fn acknowledge_options() -> Vec<QueueOption> {
    vec![QueueOption {
        key: "acknowledge".to_string(),
        label: "Acknowledge".to_string(),
        recommended: true,
    }]
}

pub fn default_ask_options() -> Vec<QueueOption> {
    vec![
        QueueOption {
            key: "allow_once".to_string(),
            label: "Allow once".to_string(),
            recommended: true,
        },
        QueueOption {
            key: "allow_always".to_string(),
            label: "Always allow in this workspace".to_string(),
            recommended: false,
        },
        QueueOption {
            key: "deny".to_string(),
            label: "Deny".to_string(),
            recommended: false,
        },
    ]
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct AttentionQueueState {
    /// True when the queue bar list (`Cmd+I`) is expanded.
    pub open: bool,
    /// Selected bar index in `items` when `open` is true.
    pub selected_idx: usize,
    /// Computed ordered list of bars (index 0 is locked when focused pane has an unresolved ask).
    pub items: Vec<QueueBar>,
    /// Synthetic / test / direct items injected into the queue.
    pub synthetic_items: Vec<QueueBar>,
    /// Currently focus-locked `(pane_id, ask_id)` pair for R38.
    pub locked_ask: Option<(u64, String)>,
    /// Dismissed non-ask bar IDs (`x` key).
    pub dismissed_ids: HashSet<String>,
    /// Answered ask IDs so each ask is decided at most once.
    pub answered_ask_ids: HashSet<String>,
    /// Recorded verdicts sent from the Decision Overlay (for tests and audit).
    pub verdicts: Vec<DecisionVerdict>,
    /// Open Decision Overlay state (`None` unless explicitly opened).
    pub decision_overlay: Option<DecisionOverlayState>,
    /// Transient status hint in the path bar (e.g. `Cmd+Alt+B` with no asks, or `Esc` hint).
    pub transient_hint: Option<(String, Instant)>,
    /// Monotonic counter for ordering arrivals when timestamps tie.
    pub next_seq: u64,
    /// Notice ids shown to the person (overlay or list opened). Once they are
    /// no longer on screen, they are acknowledged and cleared.
    pub seen_notice_ids: HashSet<String>,
}

impl AttentionQueueState {
    pub fn ask_count(&self) -> usize {
        self.items
            .iter()
            .filter(|b| b.category == QueueCategory::Ask)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|b| b.category == QueueCategory::Failed)
            .count()
    }

    pub fn total_count(&self) -> usize {
        self.items.len()
    }

    /// Unresolved asks filed in exactly `ctx_id`. The one count every surface
    /// (cards, atlas nodes, queue list) shows for a context.
    pub fn ask_count_for_context(&self, ctx_id: u64) -> usize {
        self.items
            .iter()
            .filter(|b| b.category == QueueCategory::Ask && b.context_id == ctx_id)
            .count()
    }

    pub fn ask_count_for_context_subtree(
        &self,
        root_ctx_id: u64,
        router: &crate::workspace::router::WorkspaceRouter,
    ) -> usize {
        self.items
            .iter()
            .filter(|b| {
                b.category == QueueCategory::Ask
                    && context_is_in_subtree(b.context_id, root_ctx_id, router)
            })
            .count()
    }

    pub fn first_ask_for_context(&self, ctx_id: u64) -> Option<&QueueBar> {
        self.items
            .iter()
            .find(|b| b.category == QueueCategory::Ask && b.context_id == ctx_id)
    }

    pub fn first_ask_for_pane(&self, pane_id: u64) -> Option<&QueueBar> {
        self.items
            .iter()
            .find(|b| b.category == QueueCategory::Ask && b.pane_id == pane_id)
    }
}

pub fn context_is_in_subtree(
    ctx_id: u64,
    ancestor_id: u64,
    router: &crate::workspace::router::WorkspaceRouter,
) -> bool {
    if ctx_id == ancestor_id {
        return true;
    }
    let mut cur = Some(ctx_id);
    let mut guard = 0;
    while let Some(id) = cur {
        if id == ancestor_id {
            return true;
        }
        guard += 1;
        if guard > 64 {
            break;
        }
        cur = router
            .as_slice()
            .iter()
            .find(|c| c.context_id == id)
            .and_then(|c| c.parent_id);
    }
    false
}

impl PlexiApp {
    /// Returns the currently focused pane ID at the active context/window,
    /// including `conjoined_parent_focus` when focused on a conjoined parent column.
    pub fn active_focused_pane_id(&self) -> Option<u64> {
        if let Some(parent_id) = self
            .conjoined_parent()
            .filter(|id| self.conjoined_parent_focus == Some(*id))
        {
            return Some(parent_id);
        }
        let win = self.windows.get(self.active_window)?;
        let tile = win.focused_pane?;
        let pane_tile = match win.tree.tiles.get(tile) {
            Some(egui_tiles::Tile::Pane(_)) => Some(tile),
            Some(egui_tiles::Tile::Container(_)) => win.find_first_pane_in(tile),
            None => None,
        }?;
        match win.tree.tiles.get(pane_tile) {
            Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
            _ => None,
        }
    }

    /// Inject a synthetic or test bar into the attention queue and recompute order.
    /// Never opens the decision overlay automatically (R39).
    #[allow(dead_code)]
    pub fn enqueue_attention_bar(&mut self, mut bar: QueueBar) {
        if self.attention_queue.answered_ask_ids.contains(&bar.id)
            || self.attention_queue.dismissed_ids.contains(&bar.id)
        {
            return;
        }
        // Coalesce identical unresolved asks from the same pane.
        if bar.category == QueueCategory::Ask {
            if let Some(existing) = self.attention_queue.synthetic_items.iter_mut().find(|b| {
                b.id == bar.id
                    || (b.category == QueueCategory::Ask
                        && b.pane_id == bar.pane_id
                        && !bar.command_text.is_empty()
                        && b.command_text == bar.command_text
                        && b.summary == bar.summary)
            }) {
                existing.summary = bar.summary;
                return;
            }
        } else if self
            .attention_queue
            .synthetic_items
            .iter()
            .any(|b| b.id == bar.id)
        {
            return;
        }
        if bar.options.is_empty() && bar.category == QueueCategory::Ask {
            bar.options = default_ask_options();
        }
        bar.locked = false;
        self.attention_queue.synthetic_items.push(bar);
        self.sync_attention_queue();
    }

    /// Convenience helper for tests/callers to inject an Ask for a pane.
    #[allow(dead_code)]
    pub fn enqueue_ask(
        &mut self,
        id: impl Into<String>,
        pane_id: u64,
        command_text: impl Into<String>,
        created_at_ms: u64,
        options: Vec<QueueOption>,
    ) {
        let id = id.into();
        let command_text = command_text.into();
        let (context_id, pane_name, context_path) = self.describe_pane_for_queue(pane_id);
        self.enqueue_attention_bar(QueueBar {
            id: id.clone(),
            category: QueueCategory::Ask,
            pane_id,
            pane_name,
            context_id,
            context_path,
            summary: command_text.clone(),
            command_text,
            options,
            created_at_ms,
            locked: false,
            source: QueueSource::Synthetic { id },
        });
    }

    /// Convenience helper for tests/callers to inject any queue category bar.
    #[allow(dead_code)]
    pub fn enqueue_queue_item(
        &mut self,
        id: impl Into<String>,
        category: QueueCategory,
        pane_id: u64,
        summary: impl Into<String>,
        created_at_ms: u64,
    ) {
        let id = id.into();
        let summary = summary.into();
        let (context_id, pane_name, context_path) = self.describe_pane_for_queue(pane_id);
        self.enqueue_attention_bar(QueueBar {
            id: id.clone(),
            category,
            pane_id,
            pane_name,
            context_id,
            context_path,
            summary: summary.clone(),
            command_text: summary,
            options: if category == QueueCategory::Ask {
                default_ask_options()
            } else {
                Vec::new()
            },
            created_at_ms,
            locked: false,
            source: QueueSource::Synthetic { id },
        });
    }

    pub(crate) fn describe_pane_for_queue(&self, pane_id: u64) -> (u64, String, String) {
        if let Some((win_idx, _)) = self.find_pane_in_any_window(pane_id) {
            let win = &self.windows[win_idx];
            let ctx_id = win.context_id;
            let pane_name = win
                .panes
                .get(&pane_id)
                .and_then(|p| {
                    p.as_terminal()
                        .and_then(|t| t.name.as_deref())
                        .or_else(|| p.as_app().map(|a| a.name.as_str()))
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("pane-{pane_id}"));
            let ctx_path = self.context_breadcrumb_string(ctx_id);
            (ctx_id, pane_name, ctx_path)
        } else {
            let ctx_id = self.router.active().context_id;
            (
                ctx_id,
                format!("pane-{pane_id}"),
                self.context_breadcrumb_string(ctx_id),
            )
        }
    }

    pub fn context_breadcrumb_string(&self, context_id: u64) -> String {
        let mut chain = Vec::new();
        let mut cur = Some(context_id);
        let mut guard = 0;
        while let Some(cid) = cur {
            guard += 1;
            if guard > 32 {
                break;
            }
            if let Some(ctx) = self.router.as_slice().iter().find(|c| c.context_id == cid) {
                chain.push(ctx.name.displayed().to_string());
                cur = ctx.parent_id;
            } else {
                break;
            }
        }
        chain.reverse();
        if chain.is_empty() {
            "workspace".to_string()
        } else {
            chain.join(" › ")
        }
    }

    /// Recomputes `self.attention_queue.items` from all live sources, applying:
    /// 1. Category ordering: `Ask` -> `Failed` -> `Result` -> `Done`, oldest first within each.
    /// 2. Focus-lock rule (R38): focused pane's unresolved ask stays at index 0 with `locked = true`.
    /// 3. Stale overlay check: if the open Decision Overlay's ask or pane disappeared, closes/marks it.
    pub fn sync_attention_queue(&mut self) {
        let live_context_ids: HashSet<u64> = self
            .router
            .as_slice()
            .iter()
            .map(|c| c.context_id)
            .collect();

        // Prune synthetic items whose context was dissolved.
        self.attention_queue
            .synthetic_items
            .retain(|b| b.context_id == 0 || live_context_ids.contains(&b.context_id));

        let mut bars: Vec<QueueBar> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        // 1. Synthetic / explicitly queued items.
        for item in &self.attention_queue.synthetic_items {
            if self.attention_queue.answered_ask_ids.contains(&item.id)
                || self.attention_queue.dismissed_ids.contains(&item.id)
            {
                continue;
            }
            let mut b = item.clone();
            b.locked = false;
            seen_ids.insert(b.id.clone());
            bars.push(b);
        }

        // 2. PermissionMonitor pending requests & open_needs_you items.
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        for (p_idx, pending) in monitor.list_pending().into_iter().enumerate() {
            let req_id = pending.pending_request_id.clone();
            if self.attention_queue.answered_ask_ids.contains(&req_id) || seen_ids.contains(&req_id)
            {
                continue;
            }
            let pane_id = pending
                .actor_id
                .strip_prefix("pane:")
                .and_then(|s| s.parse::<u64>().ok())
                .or(pending.instance_id)
                .or_else(|| self.active_focused_pane_id())
                .unwrap_or(1);
            let (context_id, pane_name, context_path) = self.describe_pane_for_queue(pane_id);
            let created_ms = 500_000 + p_idx as u64;
            let cmd_text = if pending.input_summary.is_empty() || pending.input_summary == "{}" {
                pending.tool.clone()
            } else {
                format!("{} — {}", pending.tool, pending.input_summary)
            };
            seen_ids.insert(req_id.clone());
            bars.push(QueueBar {
                id: req_id.clone(),
                category: QueueCategory::Ask,
                pane_id,
                pane_name,
                context_id: pending.context_id.unwrap_or(context_id),
                context_path,
                summary: format!(
                    "{} wants to use {}",
                    self.approval_actor_label(&pending.actor_id),
                    pending.tool.replace("__", " / ")
                ),
                command_text: cmd_text,
                options: default_ask_options(),
                created_at_ms: created_ms,
                locked: false,
                source: QueueSource::BrokerPending { request_id: req_id },
            });
        }

        for item in monitor.open_needs_you() {
            if self.attention_queue.answered_ask_ids.contains(&item.id)
                || self.attention_queue.dismissed_ids.contains(&item.id)
                || seen_ids.contains(&item.id)
            {
                continue;
            }
            let pane_id = item
                .details
                .as_ref()
                .and_then(|d| d.pane_id)
                .or_else(|| {
                    item.actor
                        .strip_prefix("pane:")
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .or_else(|| self.active_focused_pane_id())
                .unwrap_or(1);
            let (context_id, pane_name, context_path) = self.describe_pane_for_queue(pane_id);
            let category = if item.kind == NeedsYouKind::AgentDone {
                QueueCategory::Done
            } else if item.kind == NeedsYouKind::Integrity {
                // One notice per (kind, resource), however many restarts filed it.
                let key = format!("notice:{}", item.resource);
                if !seen_ids.insert(key) {
                    continue;
                }
                QueueCategory::Notice
            } else {
                QueueCategory::Ask
            };
            let choices = item
                .details
                .as_ref()
                .map(|d| d.choices.clone())
                .unwrap_or_default();
            let recommend = item.details.as_ref().and_then(|d| d.recommend.clone());
            let options = if !choices.is_empty() {
                choices
                    .into_iter()
                    .enumerate()
                    .map(|(idx, c)| {
                        let recommended = match recommend.as_deref() {
                            Some(r) => c == r,
                            None => c.contains("(Recommended)") || c.contains('★') || idx == 0,
                        };
                        QueueOption {
                            key: format!("choice_{}", idx + 1),
                            label: c,
                            recommended,
                        }
                    })
                    .collect()
            } else if category == QueueCategory::Ask {
                default_ask_options()
            } else if category == QueueCategory::Notice {
                acknowledge_options()
            } else {
                Vec::new()
            };
            let is_question =
                matches!(item.kind, NeedsYouKind::Question | NeedsYouKind::BlockedRun)
                    && item.details.as_ref().is_some_and(|d| !d.choices.is_empty());
            seen_ids.insert(item.id.clone());
            bars.push(QueueBar {
                id: item.id.clone(),
                category,
                pane_id,
                pane_name,
                context_id,
                context_path,
                summary: item.summary.clone(),
                command_text: if item.resource.is_empty() {
                    item.summary.clone()
                } else {
                    format!("{} — {}", item.resource, item.summary)
                },
                options,
                created_at_ms: (item.created_at.max(0) as u64).saturating_mul(1000),
                locked: false,
                source: QueueSource::NeedsYou {
                    request_id: item.id,
                    is_question,
                },
            });
        }

        // 3. Former notifications (`self.pending_notifications`) -> fed into Attention Queue (R41).
        for (n_idx, notif) in self.pending_notifications.iter().enumerate() {
            if self.notification_is_snoozed(notif) {
                continue;
            }
            let bar_id = format!("notify-{}", notif.notify_id);
            if self.attention_queue.answered_ask_ids.contains(&bar_id)
                || self.attention_queue.dismissed_ids.contains(&bar_id)
                || seen_ids.contains(&bar_id)
            {
                continue;
            }
            let src_pane = notif.agent_source_pane();
            let pane_id = if src_pane != 0 {
                src_pane
            } else {
                self.active_focused_pane_id().unwrap_or(1)
            };
            let (context_id, pane_name, context_path) = if notif.source_context_id != 0 {
                let (_, pname, _) = self.describe_pane_for_queue(pane_id);
                (
                    notif.source_context_id,
                    pname,
                    self.context_breadcrumb_string(notif.source_context_id),
                )
            } else {
                self.describe_pane_for_queue(pane_id)
            };
            let is_ask = notif.required
                || !notif.options.is_empty()
                || notif.input_prompt.is_some()
                || !matches!(notif.kind, crate::protocol::NotifyKind::Message);
            let category = if is_ask {
                QueueCategory::Ask
            } else {
                QueueCategory::Result
            };
            let options = if !notif.options.is_empty() {
                notif
                    .options
                    .iter()
                    .enumerate()
                    .map(|(idx, c)| QueueOption {
                        key: if c.value.is_empty() {
                            c.label.clone()
                        } else {
                            c.value.clone()
                        },
                        label: c.label.clone(),
                        recommended: idx == 0,
                    })
                    .collect()
            } else if is_ask {
                default_ask_options()
            } else {
                Vec::new()
            };
            let summary = if notif.body.is_empty() {
                notif.title.clone()
            } else {
                format!("{}: {}", notif.title, notif.body)
            };
            seen_ids.insert(bar_id.clone());
            bars.push(QueueBar {
                id: bar_id,
                category,
                pane_id,
                pane_name,
                context_id,
                context_path,
                summary: summary.clone(),
                command_text: summary,
                options,
                created_at_ms: 800_000 + n_idx as u64,
                locked: false,
                source: QueueSource::Notification {
                    notify_id: notif.notify_id.clone(),
                },
            });
        }

        // 4. Child result events -> QueueCategory::Result / Failed.
        for (&res_pane_id, ev) in &self.child_result_events {
            let bar_id = format!("child-result-{res_pane_id}");
            if self.attention_queue.dismissed_ids.contains(&bar_id) || seen_ids.contains(&bar_id) {
                continue;
            }
            let (context_id, pane_name, context_path) = self.describe_pane_for_queue(res_pane_id);
            let category = if ev.status.eq_ignore_ascii_case("failed")
                || ev.status.eq_ignore_ascii_case("error")
            {
                QueueCategory::Failed
            } else {
                QueueCategory::Result
            };
            let summary = ev
                .summary
                .clone()
                .unwrap_or_else(|| format!("{} ({})", ev.child_agent, ev.status));
            seen_ids.insert(bar_id.clone());
            bars.push(QueueBar {
                id: bar_id,
                category,
                pane_id: res_pane_id,
                pane_name: if ev.child_agent.is_empty() {
                    pane_name
                } else {
                    ev.child_agent.clone()
                },
                context_id,
                context_path,
                summary: summary.clone(),
                command_text: summary,
                options: Vec::new(),
                created_at_ms: ev.finished_at_epoch_secs.saturating_mul(1000),
                locked: false,
                source: QueueSource::ChildResult {
                    pane_id: res_pane_id,
                },
            });
        }

        // 5. Live pane agent states not already represented above.
        let mut agent_entries: Vec<(u64, u64, String, crate::protocol::PaneAgentState)> =
            Vec::new();
        for win in &self.windows {
            for (&pid, pane) in &win.panes {
                if let Some(ag) = pane.agent().cloned() {
                    let pname = pane
                        .as_terminal()
                        .and_then(|t| t.name.as_deref())
                        .or_else(|| pane.as_app().map(|a| a.name.as_str()))
                        .map(str::to_string)
                        .unwrap_or_else(|| ag.agent.clone());
                    agent_entries.push((win.context_id, pid, pname, ag));
                }
            }
        }
        for (ctx_id, pid, pname, ag) in agent_entries {
            let (cat, prefix) = match ag.state {
                crate::protocol::AgentState::NeedsYou => {
                    if bars
                        .iter()
                        .any(|b| b.category == QueueCategory::Ask && b.pane_id == pid)
                    {
                        continue;
                    }
                    (QueueCategory::Ask, "agent-ask")
                }
                crate::protocol::AgentState::Failed
                | crate::protocol::AgentState::Blocked
                | crate::protocol::AgentState::ExitedUnreported => {
                    if bars
                        .iter()
                        .any(|b| b.category == QueueCategory::Failed && b.pane_id == pid)
                    {
                        continue;
                    }
                    (QueueCategory::Failed, "agent-failed")
                }
                crate::protocol::AgentState::Completed => {
                    if bars.iter().any(|b| b.pane_id == pid) {
                        continue;
                    }
                    (QueueCategory::Done, "agent-done")
                }
                _ => continue,
            };
            let bar_id = format!("{prefix}-{pid}");
            if self.attention_queue.answered_ask_ids.contains(&bar_id)
                || self.attention_queue.dismissed_ids.contains(&bar_id)
                || seen_ids.contains(&bar_id)
            {
                continue;
            }
            let context_path = self.context_breadcrumb_string(ctx_id);
            let summary = ag
                .detail
                .clone()
                .unwrap_or_else(|| format!("{} {}", ag.agent, cat.label()));
            seen_ids.insert(bar_id.clone());
            bars.push(QueueBar {
                id: bar_id,
                category: cat,
                pane_id: pid,
                pane_name: pname,
                context_id: ctx_id,
                context_path,
                summary: summary.clone(),
                command_text: summary,
                options: if cat == QueueCategory::Ask {
                    default_ask_options()
                } else {
                    Vec::new()
                },
                created_at_ms: 2_000_000 + pid,
                locked: false,
                source: QueueSource::AgentState { pane_id: pid },
            });
        }

        // Sort by (category, created_at_ms) — oldest first within each category (R37).
        bars.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
        });

        // Apply the Focus-Lock Rule (R38):
        // When the focused pane has an unresolved ask, that ask is item 0 with a lock mark
        // and stays item 0 under any arrivals until answered or focus leaves the pane.
        let focused_pane = self.active_focused_pane_id();
        if let Some((locked_pane, ref locked_id)) = self.attention_queue.locked_ask {
            let still_valid = Some(locked_pane) == focused_pane
                && bars.iter().any(|b| {
                    b.category == QueueCategory::Ask
                        && b.pane_id == locked_pane
                        && &b.id == locked_id
                });
            if !still_valid {
                self.attention_queue.locked_ask = None;
            }
        }
        if self.attention_queue.locked_ask.is_none() {
            if let Some(fp) = focused_pane {
                if let Some(first_for_fp) = bars
                    .iter()
                    .find(|b| b.category == QueueCategory::Ask && b.pane_id == fp)
                {
                    self.attention_queue.locked_ask = Some((fp, first_for_fp.id.clone()));
                }
            }
        }
        if let Some((locked_pane, ref locked_id)) = self.attention_queue.locked_ask {
            if let Some(pos) = bars.iter().position(|b| {
                b.category == QueueCategory::Ask && b.pane_id == locked_pane && &b.id == locked_id
            }) {
                let mut locked_bar = bars.remove(pos);
                locked_bar.locked = true;
                bars.insert(0, locked_bar);
            }
        }

        self.attention_queue.items = bars;
        if self.attention_queue.items.is_empty() {
            self.attention_queue.selected_idx = 0;
        } else if self.attention_queue.selected_idx >= self.attention_queue.items.len() {
            self.attention_queue.selected_idx = self.attention_queue.items.len() - 1;
        }

        self.clear_seen_notices();

        // Check if an open Decision Overlay's target pane died or ask resolved externally.
        if let Some(ref overlay) = self.attention_queue.decision_overlay {
            let is_notice = self
                .attention_queue
                .items
                .iter()
                .any(|b| b.id == overlay.ask_id && b.category == QueueCategory::Notice);
            let pane_alive = is_notice || self.find_pane_in_any_window(overlay.pane_id).is_some();
            if !pane_alive {
                log::info!(
                    "decision_overlay: closing because target pane {} died",
                    overlay.pane_id
                );
                self.attention_queue.decision_overlay = None;
                self.attention_queue.transient_hint =
                    Some(("No longer needed (pane closed)".to_string(), Instant::now()));
            }
        }
    }

    /// Acknowledge every open integrity record sharing the notice's resource,
    /// so a restart loop cannot leave duplicates behind.
    fn acknowledge_notice(&mut self, notice_id: &str) {
        let resource = self
            .attention_queue
            .items
            .iter()
            .find(|b| b.id == notice_id)
            .and_then(|b| match &b.source {
                QueueSource::NeedsYou { request_id, .. } => Some(request_id.clone()),
                _ => None,
            })
            .and_then(|rid| {
                PermissionMonitor::for_profile(&crate::config::config_dir())
                    .open_needs_you()
                    .into_iter()
                    .find(|r| r.id == rid)
                    .map(|r| r.resource)
            });
        let mut ids = vec![notice_id.to_string()];
        if let Some(resource) = resource {
            ids.extend(
                PermissionMonitor::for_profile(&crate::config::config_dir())
                    .open_needs_you()
                    .into_iter()
                    .filter(|r| r.kind == NeedsYouKind::Integrity && r.resource == resource)
                    .map(|r| r.id),
            );
        }
        ids.sort();
        ids.dedup();
        for id in ids {
            if let Err(error) = self.resolve_from_desktop(&id, false, "notice acknowledge") {
                log::warn!("attention_queue: acknowledge {id} not persisted: {error}");
            }
            self.attention_queue.dismissed_ids.insert(id);
        }
        self.attention_queue.seen_notice_ids.remove(notice_id);
        log::info!("attention_queue: acknowledged notice {notice_id}");
    }

    /// Notices that were shown and are no longer on screen are acknowledged.
    fn clear_seen_notices(&mut self) {
        if self.attention_queue.open {
            self.mark_notices_seen();
        }
        if self.attention_queue.seen_notice_ids.is_empty() {
            return;
        }
        let on_screen = self
            .attention_queue
            .decision_overlay
            .as_ref()
            .map(|o| o.ask_id.clone());
        let list_open = self.attention_queue.open;
        let stale: Vec<String> = self
            .attention_queue
            .items
            .iter()
            .filter(|b| {
                b.category == QueueCategory::Notice
                    && self.attention_queue.seen_notice_ids.contains(&b.id)
                    && !list_open
                    && on_screen.as_deref() != Some(b.id.as_str())
            })
            .map(|b| b.id.clone())
            .collect();
        if stale.is_empty() {
            return;
        }
        for id in &stale {
            self.acknowledge_notice(id);
        }
        self.attention_queue
            .items
            .retain(|b| !stale.contains(&b.id));
    }

    fn mark_notices_seen(&mut self) {
        let ids: Vec<String> = self
            .attention_queue
            .items
            .iter()
            .filter(|b| b.category == QueueCategory::Notice)
            .map(|b| b.id.clone())
            .collect();
        self.attention_queue.seen_notice_ids.extend(ids);
    }

    /// Toggle the pulled-up attention queue bar list (`Cmd+I`).
    pub fn toggle_attention_queue_list(&mut self) {
        self.sync_attention_queue();
        self.attention_queue.open = !self.attention_queue.open;
        if self.attention_queue.open {
            self.mark_notices_seen();
            self.attention_queue.selected_idx = 0;
            log::info!(
                "attention_queue: opened list (items={}, asks={})",
                self.attention_queue.total_count(),
                self.attention_queue.ask_count()
            );
        } else {
            log::info!("attention_queue: closed list");
        }
    }

    /// Open the Decision Overlay for a specific ask ID, jumping to its context and pane
    /// at `ZoomLevel::Columns`.
    pub fn open_decision_overlay_for_ask(&mut self, ask_id: &str) -> bool {
        self.sync_attention_queue();
        let Some(bar) = self
            .attention_queue
            .items
            .iter()
            .find(|b| {
                b.id == ask_id
                    && matches!(b.category, QueueCategory::Ask | QueueCategory::Notice)
            })
            .cloned()
        else {
            return false;
        };
        if bar.category == QueueCategory::Notice {
            self.attention_queue.seen_notice_ids.insert(bar.id.clone());
        }
        // Host notices are not about any pane: stay where the person is.
        if bar.category != QueueCategory::Notice {
            self.jump_to_pane_at_columns(bar.pane_id);
        }
        self.attention_queue.open = false;
        self.attention_queue.decision_overlay = Some(DecisionOverlayState::from_bar(&bar));
        self.sync_attention_queue();
        log::info!(
            "decision_overlay: opened ask_id={} pane_id={}",
            bar.id,
            bar.pane_id
        );
        true
    }

    /// `Cmd+Alt+B`:
    /// - If inside the Decision Overlay: defer current ask and advance to the next ask.
    /// - Else if the focused pane has an open ask: open the Decision Overlay on it.
    /// - Else jump to the oldest ask and open the Decision Overlay.
    /// - If no asks exist: show a 1.5s hint.
    pub fn summon_decision_overlay(&mut self) {
        self.sync_attention_queue();
        let asks: Vec<QueueBar> = self
            .attention_queue
            .items
            .iter()
            .filter(|b| b.category == QueueCategory::Ask)
            .cloned()
            .collect();
        if asks.is_empty() {
            self.attention_queue.transient_hint = Some((NO_ASKS_HINT.to_string(), Instant::now()));
            log::info!("decision_overlay: Cmd+Alt+B with no open asks");
            return;
        }

        if let Some(ref current) = self.attention_queue.decision_overlay {
            let cur_id = current.ask_id.clone();
            let next_ask = asks
                .iter()
                .position(|a| a.id == cur_id)
                .map(|idx| &asks[(idx + 1) % asks.len()])
                .unwrap_or(&asks[0]);
            let next_id = next_ask.id.clone();
            let _ = self.open_decision_overlay_for_ask(&next_id);
            return;
        }

        if let Some(fp) = self.active_focused_pane_id() {
            if let Some(focused_ask) = asks.iter().find(|a| a.pane_id == fp) {
                let id = focused_ask.id.clone();
                let _ = self.open_decision_overlay_for_ask(&id);
                return;
            }
        }

        let oldest_id = asks[0].id.clone();
        let _ = self.open_decision_overlay_for_ask(&oldest_id);
    }

    /// Pick option `option_idx` in the open Decision Overlay.
    /// Sends at most one verdict, closes the overlay, and keeps focus on the pane.
    pub fn pick_decision_overlay_option(&mut self, option_idx: usize) -> bool {
        let Some(overlay) = self.attention_queue.decision_overlay.clone() else {
            return false;
        };
        let Some(option) = overlay.options.get(option_idx).cloned() else {
            return false;
        };
        if !self
            .attention_queue
            .answered_ask_ids
            .insert(overlay.ask_id.clone())
        {
            self.attention_queue.decision_overlay = None;
            return false;
        }

        // Remove from synthetic items if present.
        self.attention_queue
            .synthetic_items
            .retain(|b| b.id != overlay.ask_id);

        // Dispatch through the underlying host approval / notification channel.
        match &overlay.source {
            QueueSource::BrokerPending { request_id } => {
                let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
                let choice = match option.key.as_str() {
                    "allow_always" => ApprovalChoice::Always,
                    "deny" => ApprovalChoice::Deny,
                    _ => ApprovalChoice::Once,
                };
                match monitor.approve_pending(request_id, choice) {
                    Ok(()) => self.approval_keyboard.last_error = None,
                    Err(error) => {
                        // #2886: a failed decision must be visible and retryable.
                        // Keep the overlay open and show the error inside it.
                        log::warn!(
                            "decision_overlay: decide {request_id} option={} failed: {error}",
                            option.key
                        );
                        self.approval_keyboard.last_error = Some((request_id.clone(), error));
                        self.attention_queue
                            .answered_ask_ids
                            .remove(&overlay.ask_id);
                        return false;
                    }
                }
            }
            QueueSource::NeedsYou {
                request_id,
                is_question,
            } => {
                if option.key == "acknowledge" {
                    self.acknowledge_notice(request_id);
                } else if *is_question {
                    let _ = self.answer_from_desktop(request_id, &option.label, "decision overlay");
                } else {
                    let approve = option.key != "deny"
                        && !option.label.eq_ignore_ascii_case("deny")
                        && !option.label.eq_ignore_ascii_case("dismiss");
                    let _ = self.resolve_from_desktop(request_id, approve, "decision overlay");
                }
            }
            QueueSource::Notification { notify_id } => {
                if let Some(pos) = self
                    .pending_notifications
                    .iter()
                    .position(|n| &n.notify_id == notify_id)
                {
                    let n = self.pending_notifications.remove(pos);
                    let host_action = n
                        .options
                        .get(option_idx)
                        .and_then(|o| o.host_action.clone());
                    self.deliver_notify_action(
                        n.sender_pane_id,
                        n.notify_id,
                        option.label.clone(),
                        Some(option.key.clone()),
                        n.response_file,
                        host_action,
                    );
                    self.save_notifications();
                }
            }
            QueueSource::AgentState { pane_id } => {
                if let Some((wi, _)) = self.find_pane_in_any_window(*pane_id) {
                    if let Some(pane) = self.windows[wi].panes.get_mut(pane_id) {
                        if let Some(mut ag) = pane.agent().cloned() {
                            ag.state = crate::protocol::AgentState::Working;
                            pane.set_agent(Some(ag));
                        }
                    }
                }
            }
            QueueSource::ChildResult { .. } | QueueSource::Synthetic { .. } => {}
        }

        let verdict = DecisionVerdict {
            ask_id: overlay.ask_id.clone(),
            pane_id: overlay.pane_id,
            option_idx,
            option_key: option.key.clone(),
            option_label: option.label.clone(),
        };
        log::info!(
            "decision_overlay: verdict sent ask_id={} pane_id={} option={}:{}",
            verdict.ask_id,
            verdict.pane_id,
            verdict.option_idx,
            verdict.option_key
        );
        self.attention_queue.verdicts.push(verdict);
        self.attention_queue.decision_overlay = None;
        self.sync_attention_queue();
        true
    }

    /// Handle keyboard input when the Decision Overlay is open.
    /// Consumes all bare keys and `Cmd+Esc` / `Cmd+Alt+B`; swallows plain `Esc` with a hint.
    pub(crate) fn handle_decision_overlay_input(
        &mut self,
        input: &mut PlexiInput,
    ) -> KeyDisposition {
        if self.attention_queue.decision_overlay.is_none() {
            return KeyDisposition::Passthrough;
        }

        // Cmd+Esc closes the overlay leaving the ask queued.
        if input.consume_key(egui::Modifiers::COMMAND, Key::Escape)
            || input.consume_key(egui::Modifiers::CTRL, Key::Escape)
        {
            log::info!("decision_overlay: closed via Cmd+Esc (ask remains queued)");
            self.attention_queue.decision_overlay = None;
            input.clear();
            return KeyDisposition::Consumed;
        }

        // Cmd+Alt+B inside the overlay defers and cycles to the next ask.
        let cmd_alt = egui::Modifiers::COMMAND | egui::Modifiers::ALT;
        let ctrl_alt = egui::Modifiers::CTRL | egui::Modifiers::ALT;
        if input.consume_key(cmd_alt, Key::B) || input.consume_key(ctrl_alt, Key::B) {
            self.summon_decision_overlay();
            input.clear();
            return KeyDisposition::Consumed;
        }

        // Plain Esc is swallowed with a hint ("Cmd+Esc backs out").
        if input.consume_key(egui::Modifiers::NONE, Key::Escape) {
            if let Some(ref mut ov) = self.attention_queue.decision_overlay {
                ov.hint = Some((ESC_BACKOUT_HINT.to_string(), Instant::now()));
            }
            self.attention_queue.transient_hint =
                Some((ESC_BACKOUT_HINT.to_string(), Instant::now()));
            log::info!("decision_overlay: plain Esc swallowed with backout hint");
            input.clear();
            return KeyDisposition::Consumed;
        }

        // Check non-repeating key presses for j/k, Enter, 1-9.
        let mut move_delta: i32 = 0;
        let mut pick_enter = false;
        let mut pick_digit: Option<usize> = None;

        for ev in input.events() {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = ev
            {
                if modifiers.command || modifiers.ctrl || modifiers.alt {
                    continue;
                }
                match key {
                    Key::J | Key::ArrowDown => {
                        move_delta = 1;
                    }
                    Key::K | Key::ArrowUp => {
                        move_delta = -1;
                    }
                    Key::Enter if !*repeat => {
                        pick_enter = true;
                    }
                    Key::Num1 if !*repeat => pick_digit = Some(0),
                    Key::Num2 if !*repeat => pick_digit = Some(1),
                    Key::Num3 if !*repeat => pick_digit = Some(2),
                    Key::Num4 if !*repeat => pick_digit = Some(3),
                    Key::Num5 if !*repeat => pick_digit = Some(4),
                    Key::Num6 if !*repeat => pick_digit = Some(5),
                    Key::Num7 if !*repeat => pick_digit = Some(6),
                    Key::Num8 if !*repeat => pick_digit = Some(7),
                    Key::Num9 if !*repeat => pick_digit = Some(8),
                    _ => {}
                }
            }
        }

        if let Some(digit_idx) = pick_digit {
            let len = self
                .attention_queue
                .decision_overlay
                .as_ref()
                .map(|o| o.options.len())
                .unwrap_or(0);
            if digit_idx < len {
                self.pick_decision_overlay_option(digit_idx);
            }
            input.clear();
            return KeyDisposition::Consumed;
        }

        if pick_enter {
            let idx = self
                .attention_queue
                .decision_overlay
                .as_ref()
                .map(|o| o.selected_idx)
                .unwrap_or(0);
            self.pick_decision_overlay_option(idx);
            input.clear();
            return KeyDisposition::Consumed;
        }

        if move_delta != 0 {
            let bumped = if let Some(ref mut ov) = self.attention_queue.decision_overlay {
                let len = ov.options.len();
                if len == 0 {
                    false
                } else if move_delta > 0 {
                    if ov.selected_idx + 1 < len {
                        ov.selected_idx += 1;
                        false
                    } else {
                        true
                    }
                } else if ov.selected_idx > 0 {
                    ov.selected_idx -= 1;
                    false
                } else {
                    true
                }
            } else {
                false
            };
            if bumped {
                self.record_edge_bump(if move_delta > 0 {
                    crate::host::keys::Direction::Down
                } else {
                    crate::host::keys::Direction::Up
                });
            }
        }

        // Swallow all remaining bare keys and paste events while overlay is open.
        input.clear();
        KeyDisposition::Consumed
    }

    /// Handle keyboard input when the Attention Queue list (`Cmd+I`) is open.
    /// Bars never answer inline (`a`/`d` do nothing); `j`/`k` move selection,
    /// `Enter` jumps to the bar's pane (and opens Decision Overlay for asks),
    /// `x` dismisses non-ask bars only.
    pub(crate) fn handle_attention_queue_list_input(
        &mut self,
        input: &mut PlexiInput,
    ) -> KeyDisposition {
        if !self.attention_queue.open {
            return KeyDisposition::Passthrough;
        }
        self.sync_attention_queue();

        if input.consume_key(egui::Modifiers::COMMAND, Key::Escape)
            || input.consume_key(egui::Modifiers::CTRL, Key::Escape)
            || input.consume_key(egui::Modifiers::COMMAND, Key::I)
            || input.consume_key(egui::Modifiers::CTRL, Key::I)
        {
            self.attention_queue.open = false;
            input.clear();
            return KeyDisposition::Consumed;
        }

        let mut move_delta: i32 = 0;
        let mut press_enter = false;
        let mut press_x = false;

        for ev in input.events() {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = ev
            {
                if modifiers.command || modifiers.ctrl || modifiers.alt {
                    continue;
                }
                match key {
                    Key::J | Key::ArrowDown => move_delta = 1,
                    Key::K | Key::ArrowUp => move_delta = -1,
                    Key::Enter if !*repeat => press_enter = true,
                    Key::X if !*repeat => press_x = true,
                    _ => {}
                }
            }
        }

        if press_enter {
            if let Some(bar) = self
                .attention_queue
                .items
                .get(self.attention_queue.selected_idx)
                .cloned()
            {
                self.attention_queue.open = false;
                if bar.category == QueueCategory::Ask {
                    let _ = self.open_decision_overlay_for_ask(&bar.id);
                } else {
                    self.jump_to_pane_at_columns(bar.pane_id);
                }
            } else {
                self.attention_queue.open = false;
            }
            input.clear();
            return KeyDisposition::Consumed;
        }

        if press_x {
            if let Some(bar) = self
                .attention_queue
                .items
                .get(self.attention_queue.selected_idx)
                .cloned()
            {
                if bar.category == QueueCategory::Ask {
                    // Asks are NEVER dismissed by `x` (R37).
                    self.record_edge_bump(crate::host::keys::Direction::Right);
                } else {
                    self.dismiss_queue_bar(&bar.id);
                }
            }
            input.clear();
            return KeyDisposition::Consumed;
        }

        if move_delta != 0 {
            let len = self.attention_queue.items.len();
            if len == 0 {
                self.record_edge_bump(crate::host::keys::Direction::Down);
            } else if move_delta > 0 {
                if self.attention_queue.selected_idx + 1 < len {
                    self.attention_queue.selected_idx += 1;
                } else {
                    self.record_edge_bump(crate::host::keys::Direction::Down);
                }
            } else if self.attention_queue.selected_idx > 0 {
                self.attention_queue.selected_idx -= 1;
            } else {
                self.record_edge_bump(crate::host::keys::Direction::Up);
            }
        }

        // Unbound bare keys (including `a`, `d`, `1-9`, `Esc`) are swallowed and never answer or leak.
        input.clear();
        KeyDisposition::Consumed
    }

    /// Dismiss a non-ask bar by ID (`Failed`, `Result`, `Done`). Never dismisses an `Ask`.
    pub fn dismiss_queue_bar(&mut self, bar_id: &str) -> bool {
        self.sync_attention_queue();
        let Some(bar) = self
            .attention_queue
            .items
            .iter()
            .find(|b| b.id == bar_id)
            .cloned()
        else {
            return false;
        };
        if bar.category == QueueCategory::Ask {
            return false;
        }
        self.attention_queue.dismissed_ids.insert(bar.id.clone());
        self.attention_queue
            .synthetic_items
            .retain(|b| b.id != bar.id);
        match &bar.source {
            QueueSource::Notification { notify_id } => {
                self.pending_notifications
                    .retain(|n| &n.notify_id != notify_id);
                self.save_notifications();
            }
            QueueSource::NeedsYou { request_id, .. } => {
                let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
                let _ = monitor.resolve_needs_you_from(request_id, false, ResolveSurface::Desktop);
            }
            QueueSource::ChildResult { pane_id } => {
                self.child_result_events.retain(|&pid, _| pid != *pane_id);
            }
            QueueSource::AgentState { .. }
            | QueueSource::BrokerPending { .. }
            | QueueSource::Synthetic { .. } => {}
        }
        self.sync_attention_queue();
        true
    }

    /// Draw the pulled-up Attention Queue list (`Cmd+I`) and/or the Decision Overlay.
    pub(crate) fn draw_attention_queue_and_decision_overlay(&mut self, ctx: &egui::Context) {
        self.sync_attention_queue();

        if self.attention_queue.open {
            self.draw_attention_queue_list_panel(ctx);
        }
        if self.attention_queue.decision_overlay.is_some() {
            self.draw_decision_overlay_modal(ctx);
        }
    }

    fn draw_attention_queue_list_panel(&mut self, ctx: &egui::Context) {
        let colors = self.colors;
        let screen = ctx.Viewport_rect_fallback();

        // Scrim behind the pulled-up queue list.
        egui::Area::new(egui::Id::new("attention_queue_scrim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (_, painter) = ui.allocate_painter(screen.size(), Sense::click());
                painter.rect_filled(screen, CornerRadius::ZERO, colors.scrim_queue);
            });

        let panel_w = style::QUEUE_PANEL_W.min(screen.width() - 32.0).max(280.0);
        let pos = Pos2::new(
            screen.max.x - panel_w - style::SPACE_MD,
            screen.min.y + style::PATH_BAR_H + style::SPACE_SM,
        );

        let mut clicked_idx: Option<usize> = None;
        let mut dismiss_id: Option<String> = None;

        egui::Area::new(egui::Id::new("attention_queue_panel"))
            .order(Order::Tooltip)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(colors.terminal_bg)
                    .stroke(Stroke::new(1.5_f32, colors.border))
                    .corner_radius(style::RADIUS_MOTORWAYS)
                    .shadow(style::motorways_shadow(colors.shadow_flat))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(panel_w - 24.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Attention Queue")
                                    .font(crate::ui::theme::font_medium(style::TEXT_BODY))
                                    .color(colors.text_primary),
                            );
                            let asks = self.attention_queue.ask_count();
                            let total = self.attention_queue.total_count();
                            ui.label(
                                RichText::new(format!("◆{asks} · {total} items"))
                                    .size(style::TEXT_CAPTION)
                                    .color(if asks > 0 {
                                        colors.zoom_needs
                                    } else {
                                        colors.text_dim
                                    }),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(
                                            "j/k move · Enter jump · x dismiss · Cmd+I close",
                                        )
                                        .size(style::TEXT_HINT)
                                        .color(colors.text_dim),
                                    );
                                },
                            );
                        });
                        ui.add_space(style::SPACE_SM);

                        if self.attention_queue.items.is_empty() {
                            ui.label(
                                RichText::new("Queue is calm — no pending asks or results.")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                            return;
                        }

                        for (idx, bar) in self.attention_queue.items.iter().enumerate() {
                            let is_selected = idx == self.attention_queue.selected_idx;
                            let state = match bar.category {
                                QueueCategory::Ask => CardState::NeedsYou,
                                QueueCategory::Failed => CardState::Failed,
                                QueueCategory::Result | QueueCategory::Done => CardState::Done,
                                QueueCategory::Notice => CardState::Idle,
                            };
                            let is_ask = bar.category == QueueCategory::Ask;
                            let subtitle = if bar.locked {
                                format!("LOCKED · {}", bar.context_path)
                            } else {
                                bar.context_path.clone()
                            };
                            let spec = CardSpec {
                                state,
                                title: &bar.pane_name,
                                subtitle: &subtitle,
                                detail: &bar.summary,
                                badge: Some(if is_ask {
                                    ("Enter ▸ Decide".to_string(), colors.zoom_needs)
                                } else {
                                    ("✕".to_string(), colors.text_dim)
                                }),
                            };
                            let row_w = ui.available_width();
                            let layout = layout_card(ctx, CardForm::Bar, &spec, row_w);
                            let (row_rect, resp) = ui.allocate_exact_size(
                                Vec2::new(row_w, layout.height),
                                Sense::click(),
                            );
                            let badge_rect = paint_card(
                                &ui.painter_at(row_rect),
                                row_rect,
                                &layout,
                                &spec,
                                &colors,
                                colors.tile_bg,
                                is_selected,
                            );
                            if !is_ask {
                                let hit = ui.interact(
                                    badge_rect.expand2(Vec2::new(4.0, 6.0)),
                                    egui::Id::new(("queue_bar_dismiss", &bar.id)),
                                    Sense::click(),
                                );
                                if hit.clicked() {
                                    dismiss_id = Some(bar.id.clone());
                                }
                            }
                            if resp.clicked() {
                                clicked_idx = Some(idx);
                            }
                            ui.add_space(style::SPACE_XS);
                        }
                    });
            });

        if let Some(id) = dismiss_id {
            self.dismiss_queue_bar(&id);
        } else if let Some(idx) = clicked_idx {
            self.attention_queue.selected_idx = idx;
            if let Some(bar) = self.attention_queue.items.get(idx).cloned() {
                self.attention_queue.open = false;
                if bar.category == QueueCategory::Ask {
                    let _ = self.open_decision_overlay_for_ask(&bar.id);
                } else {
                    self.jump_to_pane_at_columns(bar.pane_id);
                }
            }
        }
    }

    fn draw_decision_overlay_modal(&mut self, ctx: &egui::Context) {
        let Some(overlay) = self.attention_queue.decision_overlay.clone() else {
            return;
        };
        let colors = self.colors;
        let screen = ctx.Viewport_rect_fallback();

        // Dark see-through layer over the dimmed pane (`rgba(12, 14, 20, 0.80)`).
        egui::Area::new(egui::Id::new("decision_overlay_scrim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (_, painter) = ui.allocate_painter(screen.size(), Sense::click());
                painter.rect_filled(screen, CornerRadius::ZERO, colors.scrim_decision);
            });

        let modal_w = style::DECISION_MODAL_W
            .min(screen.width() - 32.0)
            .max(300.0);
        let mut picked_option: Option<usize> = None;

        egui::Area::new(egui::Id::new("decision_overlay_card"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(colors.terminal_bg)
                    .stroke(Stroke::new(2.0_f32, colors.zoom_needs))
                    .corner_radius(style::RADIUS_MOTORWAYS)
                    .shadow(style::motorways_shadow(colors.shadow_flat))
                    .inner_margin(egui::Margin::symmetric(20, 18))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);
                        egui::ScrollArea::vertical()
                            .max_height((screen.height() - 120.0).max(160.0))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {

                        if let QueueSource::BrokerPending { request_id } = &overlay.source {
                            if let Some((err_id, err_msg)) = &self.approval_keyboard.last_error {
                                if err_id == request_id {
                                    ui.label(
                                        RichText::new(format!("Decision failed: {err_msg}"))
                                            .color(colors.danger)
                                            .size(style::TEXT_HINT),
                                    );
                                }
                            }
                        }

                        // Header: ◆ Needs you · pane_name · context_path
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("◆ NEEDS YOU")
                                    .font(crate::ui::theme::font_medium(style::TEXT_CAPTION))
                                    .color(colors.zoom_needs),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "· {} ({})",
                                    overlay.pane_name, overlay.context_path
                                ))
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                            );
                        });
                        ui.add_space(style::SPACE_SM);

                        // Question: wrapped in full, the overlay grows to fit.
                        let question_font = crate::ui::theme::font_medium(style::TEXT_TITLE);
                        wrapped_label(
                            ui,
                            &overlay.question,
                            question_font,
                            colors.text_primary,
                            modal_w - 12.0,
                        );
                        ui.add_space(style::SPACE_SM);

                        // Command text in monospace block
                        egui::Frame::NONE
                            .fill(colors.code_bg)
                            .stroke(Stroke::new(1.0_f32, colors.border))
                            .corner_radius(style::RADIUS_BAR)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let code_w = ui.available_width() - 12.0;
                                wrapped_label(
                                    ui,
                                    &overlay.command_text,
                                    FontId::new(style::TEXT_CAPTION, FontFamily::Monospace),
                                    colors.text_primary,
                                    code_w,
                                );
                            });
                        ui.add_space(style::SPACE_MD);

                        // 2–4 options
                        for (idx, opt) in overlay.options.iter().enumerate() {
                            let is_sel = idx == overlay.selected_idx;
                            let fill = if is_sel {
                                colors.focus_fill
                            } else {
                                colors.tile_bg
                            };
                            let stroke = if is_sel {
                                Stroke::new(1.5_f32, colors.accent)
                            } else {
                                Stroke::new(1.0_f32, colors.border)
                            };
                            let resp = egui::Frame::NONE
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(style::RADIUS_BAR)
                                .inner_margin(egui::Margin::symmetric(12, 9))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_top(|ui| {
                                        ui.label(
                                            RichText::new(format!("{}", idx + 1))
                                                .font(FontId::new(
                                                    style::TEXT_CAPTION,
                                                    FontFamily::Monospace,
                                                ))
                                                .color(colors.accent),
                                        );
                                        ui.add_space(style::SPACE_XS);
                                        let label_w = (ui.available_width()
                                            - if opt.recommended { 120.0 } else { 8.0 })
                                        .max(40.0);
                                        wrapped_label(
                                            ui,
                                            &opt.label,
                                            crate::ui::theme::font_medium(style::TEXT_BODY),
                                            colors.text_primary,
                                            label_w,
                                        );
                                        if opt.recommended {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new("★ Recommended")
                                                            .size(style::TEXT_HINT)
                                                            .color(colors.zoom_needs),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                })
                                .response
                                .interact(Sense::click());

                            let rect = resp.rect;
                            self.approval_buttons.push(crate::app::ApprovalButton {
                                label: opt.label.clone(),
                                bounds: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                                pending_request_id: overlay.ask_id.clone(),
                            });
                            if resp.clicked_by(egui::PointerButton::Primary)
                                && !self.synthetic_input_frame
                            {
                                picked_option = Some(idx);
                            }
                            ui.add_space(style::SPACE_XS);
                        }

                        ui.add_space(style::SPACE_SM);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("1–9 or j/k + Enter to pick · Cmd+Esc backs out")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            if let Some((ref hint_text, at)) = overlay.hint {
                                if at.elapsed() < Duration::from_secs(3) {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(hint_text)
                                                    .size(style::TEXT_HINT)
                                                    .color(colors.zoom_needs),
                                            );
                                        },
                                    );
                                }
                            }
                        });
                            });
                    });
            });

        if let Some(idx) = picked_option {
            self.pick_decision_overlay_option(idx);
        }
    }
}

/// Label wrapped to `max_w` by [`wrap_text`] with no line cap: every line of
/// the text is shown and the container grows to fit.
fn wrapped_label(ui: &mut egui::Ui, text: &str, font: FontId, color: egui::Color32, max_w: f32) {
    let lines = wrap_text(ui.ctx(), text, font.clone(), max_w, None);
    ui.add(
        egui::Label::new(RichText::new(lines.join("\n")).font(font).color(color))
            .wrap_mode(egui::TextWrapMode::Extend),
    );
}

trait ViewportRectFallback {
    #[allow(non_snake_case)]
    fn Viewport_rect_fallback(&self) -> Rect;
}

impl ViewportRectFallback for egui::Context {
    fn Viewport_rect_fallback(&self) -> Rect {
        self.input(|i| i.Viewport_rect())
    }
}

trait InputViewportRect {
    #[allow(non_snake_case)]
    fn Viewport_rect(&self) -> Rect;
}

impl InputViewportRect for egui::InputState {
    fn Viewport_rect(&self) -> Rect {
        self.content_rect()
    }
}

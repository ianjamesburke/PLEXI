//! Attention Queue & Decision Overlay (`SPEC.md` R35, R37, R38, R39, R41).
//!
//! Replaces the legacy notification modal, toasts, and pop-over approval banners
//! with:
//! 1. One ordered list of bars: `Ask` (amber `◆`, oldest first), `Failed` (red `✕`),
//!    `Result` (`●`), `Done` (`✓`). Bars never answer inline.
//! 2. Lock rule: when the focused pane has an unresolved ask, that ask is pinned
//!    at item 0 with a lock mark (`locked = true`) under any arrivals until
//!    answered or focus leaves the pane.
//! 3. Compact top-right mailbox icon/count in existing chrome.
//! 4. Decision Overlay: opens only on explicit user action (`Enter` on an ask bar,
//!    `Enter` on an asking atlas node, or `Cmd+Alt+B`). Never opens automatically
//!    on arrival. Shows `"What would you do here?"`, monospace command text, and
//!    2–4 options (starred recommendation when present). Sends at most one verdict.

use crate::app::input_router::PlexiInput;
use crate::app::view_trait::KeyDisposition;
use crate::app::PlexiApp;
use crate::broker::gate::{ApprovalChoice, NeedsYouKind, PermissionMonitor, ResolveSurface};
use crate::ui::card_block::{layout_card, paint_card, wrap_text, CardForm, CardSpec, CardState};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::style;
use egui::{
    Align2, CornerRadius, FontFamily, FontId, Key, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const OVERLAY_QUESTION: &str = "What would you do here?";
pub const NO_ASKS_HINT: &str = "No open asks in queue";

/// Unresolved Needs-you records from the loaded profile monitor, oldest first.
pub(crate) fn open_needs_you_items() -> Vec<crate::broker::gate::NeedsYouRecord> {
    let mut items = PermissionMonitor::loaded(&crate::config::config_dir())
        .map(|monitor| monitor.open_needs_you())
        .unwrap_or_default();
    items.sort_by_key(|item| item.created_at);
    items
}

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
        option_indices: Vec<usize>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionDestination {
    DecisionOverlay(String),
    LiveParent(crate::spatial::tiling::PaneId),
    SourcePane(crate::spatial::tiling::PaneId),
    ResultArtifact(PathBuf),
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
        let options = bar.options.clone();
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

fn pane_send_ask_options() -> Vec<QueueOption> {
    let mut options = default_ask_options();
    options[1].label = format!(
        "Allow this workspace caller any text to any pane for {} days",
        crate::broker::gate::ALWAYS_TTL_SECS / (24 * 60 * 60)
    );
    options
}

/// Choices for a typed CLI operation. The remembered choice is bound to the
/// operation, workspace, package and program, and says how long it lasts.
fn cli_operation_ask_options() -> Vec<QueueOption> {
    let mut options = default_ask_options();
    options[0].label = "Allow once".to_string();
    options[1].label = format!(
        "Remember this operation in this workspace for {} days",
        crate::broker::gate::ALWAYS_TTL_SECS / (24 * 60 * 60)
    );
    options
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
    /// A desktop notification choice awaiting the next logic pass.
    pub pending_notification_decision: Option<(String, usize)>,
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
    /// Single entry for every host-owned decision intent (digit, Enter, click).
    /// Injected input (pane send/key/click/drag, host key) shares a frame flag
    /// that is never set for desktop events; such a frame can never decide.
    fn request_decision_overlay_option(&mut self, option_idx: usize) {
        let Some(overlay) = self.attention_queue.decision_overlay.as_ref() else {
            return;
        };
        if self.synthetic_input_frame {
            log::info!(
                "decision_overlay: rejected synthetic decision ask_id={} option_idx={option_idx}",
                overlay.ask_id
            );
            return;
        }
        if matches!(overlay.source, QueueSource::Notification { .. }) {
            if self.attention_queue.pending_notification_decision.is_none() {
                self.attention_queue.pending_notification_decision =
                    Some((overlay.ask_id.clone(), option_idx));
                self.ctx.request_repaint();
            }
        } else {
            self.pick_decision_overlay_option(option_idx);
        }
    }

    pub(crate) fn service_pending_notification_decision(&mut self) -> bool {
        let Some((ask_id, option_idx)) = self.attention_queue.pending_notification_decision.take()
        else {
            return false;
        };
        if self
            .attention_queue
            .decision_overlay
            .as_ref()
            .is_some_and(|overlay| overlay.ask_id == ask_id)
        {
            self.pick_decision_overlay_option(option_idx);
        } else {
            log::info!(
                "attention_queue: queued notification decision no longer active id={ask_id}"
            );
        }
        true
    }
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
        if bar.options.is_empty() {
            if bar.category == QueueCategory::Ask {
                bar.options = default_ask_options();
            } else if bar.category == QueueCategory::Notice {
                bar.options = acknowledge_options();
            }
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
            } else if category == QueueCategory::Notice {
                acknowledge_options()
            } else {
                Vec::new()
            },
            created_at_ms,
            locked: false,
            source: QueueSource::Synthetic { id },
        });
    }

    /// Resolve `(context_id, pane_id, pane_name, context_path)` for a queue item
    /// from its optional pane and context bindings, without drifting with the
    /// currently focused window when either binding is known.
    pub(crate) fn resolve_queue_item_origin(
        &self,
        explicit_pane_id: Option<u64>,
        explicit_context_id: Option<u64>,
    ) -> (u64, u64, String, String) {
        let valid_ctx = explicit_context_id
            .filter(|&cid| cid != 0 && self.router.as_slice().iter().any(|c| c.context_id == cid));
        let valid_pane = explicit_pane_id.filter(|&pid| pid != 0);

        if let Some(pid) = valid_pane {
            if let Some((win_idx, _)) = self.find_pane_in_any_window(pid) {
                let win = &self.windows[win_idx];
                let ctx_id = win.context_id;
                let pane_name = win
                    .panes
                    .get(&pid)
                    .and_then(|p| {
                        p.as_terminal()
                            .and_then(|t| t.name.as_deref())
                            .or_else(|| p.as_view().map(|a| a.name.as_str()))
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| format!("pane-{pid}"));
                let ctx_path = self.context_breadcrumb_string(ctx_id);
                return (ctx_id, pid, pane_name, ctx_path);
            }
            let ctx_id = valid_ctx.unwrap_or(0);
            let ctx_path = self.context_breadcrumb_string(ctx_id);
            return (ctx_id, pid, format!("pane-{pid}"), ctx_path);
        }

        let ctx_id = valid_ctx.unwrap_or(0);
        (
            ctx_id,
            0,
            "Unknown source".to_string(),
            self.context_breadcrumb_string(ctx_id),
        )
    }

    pub(crate) fn describe_pane_for_queue(&self, pane_id: u64) -> (u64, String, String) {
        let (ctx_id, _, pane_name, ctx_path) = self.resolve_queue_item_origin(Some(pane_id), None);
        (ctx_id, pane_name, ctx_path)
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
            if b.context_id == 0 || !live_context_ids.contains(&b.context_id) {
                let (ctx_id, pane_id, pane_name, _) =
                    self.resolve_queue_item_origin((b.pane_id != 0).then_some(b.pane_id), None);
                b.context_id = ctx_id;
                b.pane_id = pane_id;
                if b.pane_name.is_empty() {
                    b.pane_name = pane_name;
                }
            }
            b.context_path = self.context_breadcrumb_string(b.context_id);
            b.locked = false;
            seen_ids.insert(b.id.clone());
            bars.push(b);
        }

        // 2. PermissionMonitor pending requests & open_needs_you items.
        // Display only. Outside the host there are no broker items to show.
        let monitor =
            PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap_or_else(|error| {
                log::error!("attention: no permission monitor in this process: {error}");
                PermissionMonitor::ephemeral()
            });
        for (p_idx, pending) in monitor.list_pending().into_iter().enumerate() {
            let req_id = pending.pending_request_id.clone();
            if self.attention_queue.answered_ask_ids.contains(&req_id) || seen_ids.contains(&req_id)
            {
                continue;
            }
            let explicit_ctx = pending
                .context_id
                .filter(|&id| id != 0 && live_context_ids.contains(&id));
            let explicit_pane = pending
                .actor_id
                .strip_prefix("pane:")
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| {
                    pending
                        .resource_id
                        .as_deref()
                        .and_then(|s| s.strip_prefix("pane:"))
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .or_else(|| {
                    pending.instance_id.filter(|&pid| {
                        pid != 0
                            && self.find_pane_in_any_window(pid).is_some_and(|(wi, _)| {
                                explicit_ctx.is_none_or(|cid| self.windows[wi].context_id == cid)
                            })
                    })
                });
            let (context_id, pane_id, pane_name, context_path) =
                self.resolve_queue_item_origin(explicit_pane, explicit_ctx);
            let created_ms = 500_000 + p_idx as u64;
            let is_cli_operation = pending
                .tool
                .starts_with(crate::broker::gate::CLI_OPERATION_TOOL_PREFIX);
            let cmd_text = if is_cli_operation {
                // The summary is already a full sentence naming plugin, workspace,
                // program and ambient authority; a tool-name prefix adds nothing.
                pending.input_summary.clone()
            } else if pending.input_summary.is_empty() || pending.input_summary == "{}" {
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
                context_id,
                context_path,
                summary: if monitor
                    .get_continuation(&req_id)
                    .is_some_and(|record| record.run_outcome.as_deref() == Some("retry_required"))
                {
                    format!(
                        "{} stopped waiting. Approve, then retry {}.",
                        self.approval_actor_label(&pending.actor_id),
                        pending.tool.replace("__", " / ")
                    )
                } else if is_cli_operation {
                    format!(
                        "{} wants to run a program on this computer",
                        self.approval_actor_label(&pending.actor_id)
                    )
                } else {
                    format!(
                        "{} wants to use {}",
                        self.approval_actor_label(&pending.actor_id),
                        pending.tool.replace("__", " / ")
                    )
                },
                command_text: cmd_text,
                options: if pending.tool == "pane.send" {
                    pane_send_ask_options()
                } else if is_cli_operation {
                    cli_operation_ask_options()
                } else {
                    default_ask_options()
                },
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
            let explicit_pane = item
                .details
                .as_ref()
                .and_then(|d| d.pane_id)
                .filter(|&id| id != 0)
                .or_else(|| {
                    item.actor
                        .strip_prefix("pane:")
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .or_else(|| {
                    item.resource
                        .strip_prefix("pane:")
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .or_else(|| {
                    item.run_tag
                        .as_deref()
                        .and_then(|s| s.strip_prefix("pane:"))
                        .and_then(|s| s.parse::<u64>().ok())
                });
            let explicit_ctx = item
                .details
                .as_ref()
                .and_then(|d| d.context_id)
                .filter(|&id| id != 0);
            let (context_id, mut pane_id, pane_name, context_path) =
                self.resolve_queue_item_origin(explicit_pane, explicit_ctx);
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
            let is_question =
                matches!(item.kind, NeedsYouKind::Question | NeedsYouKind::BlockedRun);
            let options = if is_question && !choices.is_empty() {
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
            } else if matches!(
                item.kind,
                NeedsYouKind::ApprovalClick
                    | NeedsYouKind::PermissionChange
                    | NeedsYouKind::PermissionRequest
                    | NeedsYouKind::Keychain
            ) {
                vec![
                    QueueOption {
                        key: "approve".into(),
                        label: "Approve".into(),
                        recommended: false,
                    },
                    QueueOption {
                        key: "deny".into(),
                        label: "Deny".into(),
                        recommended: false,
                    },
                ]
            } else if category == QueueCategory::Notice {
                acknowledge_options()
            } else {
                Vec::new()
            };
            if options.is_empty() && explicit_pane.is_none() {
                pane_id = 0;
            }
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
                    is_question: is_question
                        && item.details.as_ref().is_some_and(|d| !d.choices.is_empty()),
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
            let explicit_pane = (src_pane != 0).then_some(src_pane);
            let explicit_ctx = (notif.source_context_id != 0).then_some(notif.source_context_id);
            let (context_id, mut pane_id, pane_name, context_path) =
                self.resolve_queue_item_origin(explicit_pane, explicit_ctx);
            let is_ask = notif.required
                || !notif.options.is_empty()
                || notif.input_prompt.is_some()
                || !matches!(notif.kind, crate::protocol::NotifyKind::Message);
            let category = if is_ask {
                QueueCategory::Ask
            } else {
                QueueCategory::Result
            };
            let plugin_receiver = !notif.tombstoned
                && self
                    .windows
                    .iter()
                    .find_map(|win| win.panes.get(&notif.sender_pane_id))
                    .and_then(|pane| pane.as_view())
                    .is_some_and(|app| app.runtime.supports_notify_action());
            let response_receiver = notif.response_file.is_some();
            let uncertain = notif.delivery_unknown;
            let option_indices: Vec<usize> = notif
                .options
                .iter()
                .enumerate()
                .filter_map(|(idx, option)| {
                    let host_receiver = option
                        .host_action
                        .as_deref()
                        .and_then(|action| action.strip_prefix("pane_focus:"))
                        .and_then(|id| id.parse::<u64>().ok())
                        .is_some_and(|id| self.find_pane_in_any_window(id).is_some());
                    (!uncertain && (plugin_receiver || response_receiver || host_receiver))
                        .then_some(idx)
                })
                .collect();
            let options = if !option_indices.is_empty() {
                option_indices
                    .iter()
                    .enumerate()
                    .map(|(idx, source_idx)| {
                        let c = &notif.options[*source_idx];
                        QueueOption {
                            key: if c.value.is_empty() {
                                c.label.clone()
                            } else {
                                c.value.clone()
                            },
                            label: c.label.clone(),
                            recommended: idx == 0,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if is_ask && options.is_empty() && explicit_pane.is_none() {
                pane_id = 0;
            }
            let summary = if notif.body.is_empty() {
                notif.title.clone()
            } else {
                format!("{}: {}", notif.title, notif.body)
            };
            let summary = if uncertain {
                format!("Outcome unknown — {summary}")
            } else {
                summary
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
                    option_indices,
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
                        .or_else(|| pane.as_view().map(|a| a.name.as_str()))
                        .map(str::to_string)
                        .unwrap_or_else(|| ag.agent.clone());
                    agent_entries.push((win.context_id, pid, pname, ag));
                }
            }
        }
        for (ctx_id, pid, pname, ag) in agent_entries {
            if !self.attention_queue.dismissed_ids.is_empty()
                && !matches!(
                    ag.state,
                    crate::protocol::AgentState::Failed
                        | crate::protocol::AgentState::Blocked
                        | crate::protocol::AgentState::ExitedUnreported
                )
            {
                self.attention_queue
                    .dismissed_ids
                    .remove(&format!("agent-failed-{pid}"));
            }
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
                    if bars.iter().any(|b| {
                        b.pane_id == pid
                            && (b.category == QueueCategory::Failed
                                || (ag.state == crate::protocol::AgentState::Blocked
                                    && b.category == QueueCategory::Ask))
                    }) {
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
                options: Vec::new(),
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
            let independent_notification_route =
                matches!(overlay.source, QueueSource::Notification { .. })
                    && self
                        .attention_queue
                        .items
                        .iter()
                        .any(|bar| bar.id == overlay.ask_id && !bar.options.is_empty());
            let unbound_host_decision = overlay.pane_id == 0
                && matches!(
                    overlay.source,
                    QueueSource::BrokerPending { .. } | QueueSource::NeedsYou { .. }
                )
                && self
                    .attention_queue
                    .items
                    .iter()
                    .any(|bar| bar.id == overlay.ask_id && !bar.options.is_empty());
            let uncertain_notification = match &overlay.source {
                QueueSource::Notification { notify_id, .. } => self
                    .pending_notifications
                    .iter()
                    .any(|n| &n.notify_id == notify_id && n.delivery_unknown),
                _ => false,
            };
            let pane_alive = is_notice
                || independent_notification_route
                || unbound_host_decision
                || uncertain_notification
                || self.find_pane_in_any_window(overlay.pane_id).is_some();
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
                    .ok()?
                    .open_needs_you()
                    .into_iter()
                    .find(|r| r.id == rid)
                    .map(|r| r.resource)
            });
        let mut ids = vec![notice_id.to_string()];
        if let Some(resource) = resource {
            if let Ok(monitor) = PermissionMonitor::for_profile(&crate::config::config_dir()) {
                ids.extend(
                    monitor
                        .open_needs_you()
                        .into_iter()
                        .filter(|r| r.kind == NeedsYouKind::Integrity && r.resource == resource)
                        .map(|r| r.id),
                );
            }
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
            self.sync_attention_queue();
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
                b.id == ask_id && matches!(b.category, QueueCategory::Ask | QueueCategory::Notice)
            })
            .cloned()
        else {
            return false;
        };
        if bar.category == QueueCategory::Ask && bar.options.is_empty() {
            self.attention_queue.decision_overlay = None;
            self.activate_attention_bar(&bar);
            return true;
        }
        if bar.category == QueueCategory::Notice {
            self.attention_queue.seen_notice_ids.insert(bar.id.clone());
        }
        // Host notices are not about any pane: stay where the person is.
        if bar.category != QueueCategory::Notice && bar.pane_id != 0 {
            self.jump_to_pane_at_columns(bar.pane_id);
        }
        self.attention_queue.decision_overlay = Some(DecisionOverlayState::from_bar(&bar));
        self.attention_queue.open = false;
        self.sync_attention_queue();
        log::info!(
            "decision_overlay: opened ask_id={} pane_id={}",
            bar.id,
            bar.pane_id
        );
        true
    }

    pub(crate) fn find_parent_pane_id(&self, pane_id: u64) -> Option<u64> {
        if let Some(t) = self
            .windows
            .iter()
            .find_map(|w| w.panes.get(&pane_id))
            .and_then(|p| p.as_terminal())
        {
            if t.spawned_by.as_deref() == Some("human") {
                return None;
            }
            if let Some(pid) = t.parent_pane_id {
                return Some(pid);
            }
        }

        if let Some(record) = self
            .fleet_history
            .borrow()
            .iter()
            .rev()
            .find(|r| r.pane_id == pane_id)
        {
            if record.spawned_by.as_deref() == Some("human") {
                return None;
            }
            if let Some(pid) = record.parent_pane_id {
                return Some(pid);
            }
        }

        if let Some(&pid) = self.child_parent_panes.get(&pane_id) {
            return Some(pid);
        }

        None
    }

    pub(crate) fn find_agent_result_artifact_path(&self, pane_id: u64) -> Option<PathBuf> {
        // 1. Check workspace directories from live windows
        for w in &self.windows {
            let md = w
                .path
                .join(".plexi")
                .join("results")
                .join(pane_id.to_string())
                .join("RESULT.md");
            if md.is_file() {
                return Some(md);
            }
        }

        // 2. Check fleet history closed runs
        if let Some(r) = self
            .fleet_history
            .borrow()
            .iter()
            .rev()
            .find(|r| r.pane_id == pane_id)
        {
            let md = r
                .cwd
                .join(".plexi")
                .join("results")
                .join(pane_id.to_string())
                .join("RESULT.md");
            if md.is_file() {
                return Some(md);
            }
        }

        // 3. Check profile results directory
        let profile_md = crate::config::config_dir()
            .join("results")
            .join(pane_id.to_string())
            .join("RESULT.md");
        if profile_md.is_file() {
            return Some(profile_md);
        }

        // 4. Check NeedsYou records with preview_refs
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .ok()?;
        for rec in monitor.open_needs_you() {
            let rec_pid = rec.details.as_ref().and_then(|d| d.pane_id).or_else(|| {
                rec.actor
                    .strip_prefix("pane:")
                    .and_then(|s| s.parse::<u64>().ok())
            });
            if rec_pid == Some(pane_id) {
                if let Some(p_str) = rec.details.as_ref().and_then(|d| d.preview_refs.first()) {
                    let p = PathBuf::from(p_str);
                    if p.is_file() {
                        return Some(p);
                    }
                }
                if !rec.resource.starts_with("pane:") && !rec.resource.is_empty() {
                    let p = PathBuf::from(&rec.resource);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }

        // 5. Try resolving through read_persisted_agent_result_in
        if let Ok((json_p, _, _)) = crate::cli::agent::read_persisted_agent_result_in(
            Some(&pane_id.to_string()),
            None,
            &crate::config::config_dir(),
        ) {
            let md = json_p.parent().unwrap_or(Path::new("")).join("RESULT.md");
            if md.is_file() {
                return Some(md);
            }
        }

        None
    }

    /// Profile-scoped fallback used when no persisted result artifact remains.
    ///
    /// This only identifies the fallback; it must stay side-effect-free because
    /// destination resolution is also used for preview and keyboard routing.
    fn fallback_result_artifact_path(&self, pane_id: u64) -> PathBuf {
        crate::config::config_dir()
            .join("results")
            .join(pane_id.to_string())
            .join("RESULT.md")
    }

    /// Materialize the fallback only when the person activates it for viewing.
    fn ensure_fallback_result_artifact(&self, pane_id: u64, path: &Path) -> std::io::Result<()> {
        if path.is_file() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let summary_str = self
            .child_result_events
            .get(&pane_id)
            .and_then(|ev| ev.summary.clone())
            .unwrap_or_else(|| format!("Result for pane {pane_id}"));
        let content = format!("# Agent Result\n\nPane: {pane_id}\n\n{summary_str}\n");
        std::fs::write(path, content)
    }

    /// Resolve the navigation destination for an attention queue item.
    ///
    /// For asks and notices, returns `DecisionOverlay`.
    /// For results:
    /// - If a live parent agent exists, returns `LiveParent(parent_id)` even if child is closed.
    /// - Otherwise (absent parent, non-agent parent, or human-started), falls back to:
    ///   - `SourcePane(pane_id)` if the source pane is still live.
    ///   - `ResultArtifact(path)` if the source pane is closed.
    pub fn resolve_attention_destination(&self, bar: &QueueBar) -> AttentionDestination {
        if bar.category == QueueCategory::Ask && bar.options.is_empty() {
            return AttentionDestination::SourcePane(bar.pane_id);
        }
        if matches!(bar.category, QueueCategory::Ask | QueueCategory::Notice) {
            return AttentionDestination::DecisionOverlay(bar.id.clone());
        }
        if bar.pane_id == 0 {
            return AttentionDestination::SourcePane(0);
        }

        if let Some(parent_id) = self.find_parent_pane_id(bar.pane_id) {
            if let Some(parent_pane) = self.windows.iter().find_map(|w| w.panes.get(&parent_id)) {
                if parent_pane.is_agent_pane() {
                    return AttentionDestination::LiveParent(parent_id);
                }
            }
        }

        if self.find_pane_in_any_window(bar.pane_id).is_some() {
            return AttentionDestination::SourcePane(bar.pane_id);
        }

        AttentionDestination::ResultArtifact(
            self.find_agent_result_artifact_path(bar.pane_id)
                .unwrap_or_else(|| self.fallback_result_artifact_path(bar.pane_id)),
        )
    }

    /// Single destination helper for mouse and keyboard activation from attention items.
    ///
    /// For asks and notices, opens the Decision Overlay.
    /// For results, navigates to the resolved destination (live parent, source pane,
    /// or result artifact) while leaving the child result reachable in the queue.
    pub fn activate_attention_bar(&mut self, bar: &QueueBar) -> AttentionDestination {
        let dest = self.resolve_attention_destination(bar);
        log::info!(
            "attention_queue: activating bar id={} category={:?} dest={:?}",
            bar.id,
            bar.category,
            dest
        );

        match &dest {
            AttentionDestination::DecisionOverlay(ask_id) => {
                self.open_decision_overlay_for_ask(ask_id);
            }
            AttentionDestination::LiveParent(parent_id) => {
                self.attention_queue.open = false;
                self.jump_to_pane_at_columns(*parent_id);
                log::info!(
                    "attention_queue: navigated result bar_id={} to live parent pane_id={}",
                    bar.id,
                    parent_id
                );
            }
            AttentionDestination::SourcePane(source_id) => {
                if self.find_pane_in_any_window(*source_id).is_none() {
                    self.attention_queue.transient_hint = Some((
                        "Source pane is no longer available".to_string(),
                        Instant::now(),
                    ));
                    log::info!(
                        "attention_queue: source pane unavailable bar_id={} pane_id={}",
                        bar.id,
                        source_id
                    );
                    return dest;
                }
                self.attention_queue.open = false;
                let navigated = self.jump_to_pane_at_columns(*source_id);
                if navigated
                    && bar.category == QueueCategory::Failed
                    && self.dismiss_queue_bar(&bar.id)
                {
                    log::info!(
                        "attention_queue: dismissed failed bar on activation id={}",
                        bar.id
                    );
                }
                log::info!(
                    "attention_queue: source pane navigation bar_id={} pane_id={} succeeded={}",
                    bar.id,
                    source_id,
                    navigated
                );
            }
            AttentionDestination::ResultArtifact(path) => {
                self.attention_queue.open = false;
                if bar.context_id != 0 && self.router.active().context_id != bar.context_id {
                    self.switch_to_context_restoring_last_pane(bar.context_id);
                }
                if let Err(error) = self.ensure_fallback_result_artifact(bar.pane_id, path) {
                    log::warn!(
                        "attention_queue: failed to materialize fallback result artifact {} for bar_id={}: {error}",
                        path.display(),
                        bar.id
                    );
                    return dest;
                }
                let path_str = path.to_string_lossy().into_owned();
                if let Err(e) = self.launch_view_by_id_with_layout(
                    "text-editor",
                    Some("split_h".into()),
                    &[path_str],
                    None,
                ) {
                    log::warn!(
                        "attention_queue: failed to open result viewer for {}: {e}",
                        path.display()
                    );
                } else {
                    log::info!(
                        "attention_queue: opened result artifact {} for bar_id={}",
                        path.display(),
                        bar.id
                    );
                }
            }
        }

        dest
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
                let monitor = match PermissionMonitor::for_profile(&crate::config::config_dir()) {
                    Ok(monitor) => monitor,
                    Err(error) => {
                        log::error!("decision_overlay: {error}");
                        self.approval_keyboard.last_error =
                            Some((request_id.clone(), error.to_string()));
                        return false;
                    }
                };
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
                } else {
                    let result = if *is_question {
                        self.answer_from_desktop(request_id, &option.label, "decision overlay")
                    } else {
                        let approve = option.key != "deny"
                            && !option.label.eq_ignore_ascii_case("deny")
                            && !option.label.eq_ignore_ascii_case("dismiss");
                        self.resolve_from_desktop(request_id, approve, "decision overlay")
                    };
                    match result {
                        Ok(()) => self.approval_keyboard.last_error = None,
                        Err(error) => {
                            log::info!(
                                "decision_overlay: needs_you decision failed id={request_id}; retaining overlay for review: {error}"
                            );
                            self.approval_keyboard.last_error = Some((request_id.clone(), error));
                            self.attention_queue
                                .answered_ask_ids
                                .remove(&overlay.ask_id);
                            return false;
                        }
                    }
                }
            }
            QueueSource::Notification {
                notify_id,
                option_indices,
            } => {
                if self
                    .pending_notifications
                    .iter()
                    .any(|n| &n.notify_id == notify_id && n.delivery_unknown)
                {
                    self.attention_queue
                        .answered_ask_ids
                        .remove(&overlay.ask_id);
                    self.approval_keyboard.last_error = Some((overlay.ask_id.clone(), "The action may already have happened. Check the recipient before taking further action.".into()));
                    return false;
                }
                if let Some(pos) = self
                    .pending_notifications
                    .iter()
                    .position(|n| &n.notify_id == notify_id)
                {
                    self.pending_notifications[pos].delivery_unknown = true;
                    if let Err(error) = self.try_save_notifications() {
                        self.pending_notifications[pos].delivery_unknown = false;
                        self.attention_queue
                            .answered_ask_ids
                            .remove(&overlay.ask_id);
                        self.approval_keyboard.last_error = Some((
                            overlay.ask_id.clone(),
                            format!("Could not record this decision; no action was sent: {error}"),
                        ));
                        log::warn!("decision_overlay: notification attempt not recorded id={notify_id}: {error}");
                        return false;
                    }
                    let n = self.pending_notifications.remove(pos);
                    let host_action = n
                        .options
                        .get(
                            option_indices
                                .get(option_idx)
                                .copied()
                                .unwrap_or(usize::MAX),
                        )
                        .and_then(|o| o.host_action.clone());
                    let delivered = self.deliver_notify_action(
                        n.sender_pane_id,
                        n.notify_id.clone(),
                        option.label.clone(),
                        Some(option.key.clone()),
                        n.response_file.clone(),
                        host_action,
                    );
                    if delivered != crate::app::notifications::NotifyDelivery::Delivered {
                        self.pending_notifications.insert(pos, n);
                        self.attention_queue
                            .answered_ask_ids
                            .remove(&overlay.ask_id);
                        let message = match delivered {
                            crate::app::notifications::NotifyDelivery::Unavailable => {
                                self.pending_notifications[pos].delivery_unknown = false;
                                if let Err(error) = self.try_save_notifications() {
                                    self.pending_notifications[pos].delivery_unknown = true;
                                    log::warn!("decision_overlay: could not persist unavailable outcome id={notify_id}: {error}");
                                    "The outcome could not be saved. Check the recipient before taking further action."
                                } else {
                                    "The recipient is no longer available."
                                }
                            },
                            crate::app::notifications::NotifyDelivery::OutcomeUnknown => {
                                "The action may already have happened. Check the recipient before taking further action."
                            },
                            crate::app::notifications::NotifyDelivery::Delivered => "The response was delivered.",
                        };
                        self.approval_keyboard.last_error =
                            Some((overlay.ask_id.clone(), message.into()));
                        log::warn!("decision_overlay: notification response failed id={notify_id}; outcome={delivered:?}");
                        return false;
                    }
                    self.save_notifications();
                } else {
                    self.attention_queue
                        .answered_ask_ids
                        .remove(&overlay.ask_id);
                    self.approval_keyboard.last_error = Some((
                        overlay.ask_id.clone(),
                        "This request is no longer available.".into(),
                    ));
                    log::info!("decision_overlay: notification no longer available id={notify_id}");
                    return false;
                }
            }
            QueueSource::AgentState { pane_id } => {
                log::warn!(
                    "decision_overlay: agent state has no response endpoint pane_id={pane_id}"
                );
                self.attention_queue
                    .answered_ask_ids
                    .remove(&overlay.ask_id);
                return false;
            }
            QueueSource::ChildResult { .. } | QueueSource::Synthetic { .. } => {
                if option.key == "acknowledge" {
                    self.acknowledge_notice(&overlay.ask_id);
                }
            }
        }

        let verdict = DecisionVerdict {
            ask_id: overlay.ask_id.clone(),
            pane_id: overlay.pane_id,
            option_idx,
            option_key: option.key.clone(),
            option_label: option.label.clone(),
        };
        log::info!(
            "decision_overlay: host decision recorded ask_id={} pane_id={} option={}:{}",
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
    /// Consumes all bare keys and `Esc` / `Cmd+Esc` / `Cmd+Alt+B`.
    pub(crate) fn handle_decision_overlay_input(
        &mut self,
        input: &mut PlexiInput,
    ) -> KeyDisposition {
        if self.attention_queue.decision_overlay.is_none() {
            return KeyDisposition::Passthrough;
        }
        if self.synthetic_input_frame {
            input.clear();
            log::info!("decision_overlay: rejected synthetic keyboard input");
            return KeyDisposition::Consumed;
        }

        // Plain Esc (and Cmd+Esc / Ctrl+Esc alias) closes the overlay leaving the ask queued.
        if input.consume_key(egui::Modifiers::NONE, Key::Escape)
            || input.consume_key(egui::Modifiers::COMMAND, Key::Escape)
            || input.consume_key(egui::Modifiers::CTRL, Key::Escape)
        {
            log::info!("decision_overlay: closed via Esc (ask remains queued)");
            self.attention_queue.decision_overlay = None;
            self.sync_attention_queue();
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
                self.request_decision_overlay_option(digit_idx);
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
            self.request_decision_overlay_option(idx);
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

        if input.consume_key(egui::Modifiers::NONE, Key::Escape)
            || input.consume_key(egui::Modifiers::COMMAND, Key::Escape)
            || input.consume_key(egui::Modifiers::CTRL, Key::Escape)
            || input.consume_key(egui::Modifiers::COMMAND, Key::I)
            || input.consume_key(egui::Modifiers::CTRL, Key::I)
        {
            self.attention_queue.open = false;
            self.sync_attention_queue();
            log::info!("attention_queue: closed list");
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
                self.activate_attention_bar(&bar);
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

        // Unbound bare keys (including `a`, `d`, `1-9`) are swallowed and never answer or leak.
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
            QueueSource::Notification { notify_id, .. } => {
                self.pending_notifications
                    .retain(|n| &n.notify_id != notify_id);
                self.save_notifications();
            }
            QueueSource::NeedsYou { request_id, .. } => {
                match PermissionMonitor::for_profile(&crate::config::config_dir()) {
                    Ok(monitor) => {
                        let _ = monitor.resolve_needs_you_from(
                            request_id,
                            false,
                            ResolveSurface::Desktop,
                        );
                    }
                    Err(error) => log::error!("attention: dismiss left item open: {error}"),
                }
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
        let screen = ctx.content_rect();

        let mut scrim_clicked = false;

        // Scrim behind the pulled-up queue list, positioned below the toolbar
        // so the top chrome and mailbox button remain interactively accessible.
        let scrim_rect = Rect::from_min_max(
            Pos2::new(
                screen.min.x,
                screen.min.y + crate::platform::titlebar::toolbar_height(ctx),
            ),
            screen.max,
        );
        egui::Area::new(egui::Id::new("attention_queue_scrim"))
            .order(Order::Foreground)
            .fixed_pos(scrim_rect.min)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(scrim_rect, CornerRadius::ZERO, colors.scrim_queue);
                if ui.allocate_rect(scrim_rect, egui::Sense::click()).clicked() {
                    scrim_clicked = true;
                }
            });

        let panel_w = style::QUEUE_PANEL_W.min(screen.width() - 32.0).max(280.0);
        let pos = Pos2::new(
            screen.max.x - panel_w - style::SPACE_MD,
            screen.min.y
                + (style::PATH_BAR_H + style::SPACE_SM)
                    .max(crate::platform::titlebar::toolbar_height(ctx) + style::SPACE_SM),
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
                                RichText::new(format_attention_queue_header_count(asks, total))
                                    .size(style::TEXT_CAPTION)
                                    .color(if asks > 0 {
                                        colors.zoom_needs
                                    } else {
                                        colors.text_dim
                                    }),
                            );
                            HintBar::new(&[
                                HintGroup::alternatives(&[&["j"], &["k"]], "move"),
                                HintGroup::new(&["Enter"], "jump"),
                                HintGroup::new(&["x"], "dismiss"),
                                HintGroup::new(&["Esc"], "close"),
                            ])
                            .inline_status_bar()
                            .show(ui, &colors);
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
                                    (
                                        format!(
                                            "{} ▸ {}",
                                            crate::ui::shortcuts::key_text(&["Enter"]),
                                            if bar.options.is_empty() {
                                                if self
                                                    .find_pane_in_any_window(bar.pane_id)
                                                    .is_some()
                                                {
                                                    "Open pane"
                                                } else {
                                                    "Pane unavailable"
                                                }
                                            } else {
                                                "Decide"
                                            }
                                        ),
                                        colors.zoom_needs,
                                    )
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

        if scrim_clicked {
            self.toggle_attention_queue_list();
            return;
        }

        if let Some(id) = dismiss_id {
            self.dismiss_queue_bar(&id);
        } else if let Some(idx) = clicked_idx {
            self.attention_queue.selected_idx = idx;
            if let Some(bar) = self.attention_queue.items.get(idx).cloned() {
                self.activate_attention_bar(&bar);
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

        let v_margin = 18_i8;
        let max_h = (screen.height() - 2.0 * (style::SPACE_MD + f32::from(v_margin))).max(160.0);

        egui::Area::new(egui::Id::new("decision_overlay_card"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fade_in(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(colors.terminal_bg)
                    .stroke(Stroke::new(2.0_f32, colors.zoom_needs))
                    .corner_radius(style::RADIUS_MOTORWAYS)
                    .shadow(style::motorways_shadow(colors.shadow_flat))
                    .inner_margin(egui::Margin::symmetric(20, v_margin))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);
                        let scroll_out = egui::ScrollArea::vertical()
                            .id_salt("decision_overlay_scroll")
                            .max_height(max_h)
                            .min_scrolled_height(max_h)
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                            )
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.set_width(modal_w);

                                if let Some((err_id, err_msg)) = &self.approval_keyboard.last_error
                                {
                                    if err_id == &overlay.ask_id {
                                        ui.label(
                                            RichText::new(format!("Decision failed: {err_msg}"))
                                                .color(colors.danger)
                                                .size(style::TEXT_HINT),
                                        );
                                    }
                                }

                                // Header: ◆ Needs you · pane_name · context_path
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("◆ NEEDS YOU")
                                            .font(crate::ui::theme::font_medium(
                                                style::TEXT_CAPTION,
                                            ))
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
                                let question_font =
                                    crate::ui::theme::font_medium(style::TEXT_TITLE);
                                wrapped_label(
                                    ui,
                                    &overlay.question,
                                    question_font,
                                    colors.text_primary,
                                    modal_w,
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
                                        let code_w = ui.available_width();
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
                                        // Same width as the selected stroke so selection never resizes the row.
                                        Stroke::new(1.5_f32, colors.border)
                                    };
                                    let resp = egui::Frame::NONE
                                        .fill(fill)
                                        .stroke(stroke)
                                        .corner_radius(style::RADIUS_BAR)
                                        .inner_margin(egui::Margin::symmetric(12, 9))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            paint_option_row(ui, idx + 1, opt, &colors);
                                        })
                                        .response
                                        .interact(Sense::click());

                                    let rect = resp.rect;
                                    self.approval_buttons.push(crate::app::ApprovalButton {
                                        label: opt.label.clone(),
                                        bounds: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                                        pending_request_id: overlay.ask_id.clone(),
                                    });
                                    if resp.clicked_by(egui::PointerButton::Primary) {
                                        picked_option = Some(idx);
                                    }
                                    ui.add_space(style::SPACE_XS);
                                }

                                ui.add_space(style::SPACE_SM);
                                if let Some((ref hint_text, at)) = overlay.hint {
                                    if at.elapsed() < Duration::from_secs(3) {
                                        ui.label(
                                            RichText::new(hint_text)
                                                .size(style::TEXT_HINT)
                                                .color(colors.zoom_needs),
                                        );
                                    }
                                }
                                HintBar::new(&[
                                    HintGroup::new(&["1\u{2013}9"], "pick"),
                                    HintGroup::alternatives(&[&["j"], &["k"]], "move"),
                                    HintGroup::new(&["Enter"], "confirm"),
                                    HintGroup::new(&["Esc"], "close"),
                                ])
                                .show(ui, &colors);
                            });
                        if scroll_out.content_size.y <= scroll_out.inner_rect.height() + 0.5 {
                            ui.ctx().data_mut(|d| {
                                d.remove::<egui::scroll_area::State>(scroll_out.id);
                            });
                        }
                        let card_inner_size = ui.min_rect().size();
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(
                                egui::Id::new("decision_overlay_scroll_metrics"),
                                (
                                    scroll_out.content_size,
                                    scroll_out.inner_rect.size(),
                                    card_inner_size,
                                    scroll_out.id,
                                ),
                            );
                        });
                    });
            });

        if let Some(idx) = picked_option {
            self.request_decision_overlay_option(idx);
        }
    }
}

/// Format the count summary in the Attention Queue header so the same number
/// is never printed twice (`asks == total` collapses to `◆N items`, `asks == 0`
/// omits `◆0`, and `0 < asks < total` shows `◆asks · total items`).
pub(crate) fn format_attention_queue_header_count(asks: usize, total: usize) -> String {
    let noun = if total == 1 { "item" } else { "items" };
    if total == 0 {
        "0 items".to_string()
    } else if asks == 0 {
        format!("{total} {noun}")
    } else if asks >= total {
        let ask_noun = if asks == 1 { "item" } else { "items" };
        format!("◆{asks} {ask_noun}")
    } else {
        format!("◆{asks} · {total} {noun}")
    }
}

/// Visual state for the compact top-right attention mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionMailboxVisualState {
    /// No open items; queue is calm.
    Empty,
    /// One or more open asks require user decisions.
    AsksPresent,
    /// No open asks, but one or more failures require attention.
    Failed,
    /// No asks or failures, but one or more results or notices are ready.
    ResultOrNotice,
}

/// Format the compact mailbox icon and count label for the top-right chrome.
///
/// Returns `(icon, count_text, visual_state)`.
/// - When `total == 0`: `("📭", "0", Empty)`
/// - When `asks > 0`: `("📬", count, AsksPresent)` where count is `{asks}` (when asks == total)
///   or `{asks} · {total}` (when additional non-ask items are present).
/// - When `asks == 0 && failed > 0`: `("📬", "{total}", Failed)`
/// - When `asks == 0 && total > 0`: `("📬", "{total}", ResultOrNotice)`
pub(crate) fn format_attention_mailbox(
    asks: usize,
    failed: usize,
    total: usize,
) -> (&'static str, String, AttentionMailboxVisualState) {
    if total == 0 {
        (
            "\u{1F4ED}",
            "0".to_string(),
            AttentionMailboxVisualState::Empty,
        )
    } else if asks > 0 {
        let count_str = if asks >= total {
            crate::ui::text::compact_count(asks)
        } else {
            format!(
                "{} \u{00B7} {}",
                crate::ui::text::compact_count(asks),
                crate::ui::text::compact_count(total)
            )
        };
        (
            "\u{1F4EC}",
            count_str,
            AttentionMailboxVisualState::AsksPresent,
        )
    } else if failed > 0 {
        (
            "\u{1F4EC}",
            crate::ui::text::compact_count(total),
            AttentionMailboxVisualState::Failed,
        )
    } else {
        (
            "\u{1F4EC}",
            crate::ui::text::compact_count(total),
            AttentionMailboxVisualState::ResultOrNotice,
        )
    }
}

/// One decision-overlay option row body: index, label, and (when recommended)
/// an inline right-aligned tag. Every row is exactly one body-text line tall;
/// the label ellipsizes to the width the tag leaves, so the tag never grows
/// the row.
fn paint_option_row(
    ui: &mut egui::Ui,
    number: usize,
    opt: &QueueOption,
    colors: &crate::ui::theme::Colors,
) {
    let body_font = crate::ui::theme::font_medium(style::TEXT_BODY);
    let num_font = FontId::new(style::TEXT_CAPTION, FontFamily::Monospace);
    let tag_font = FontId::proportional(style::TEXT_HINT);
    let row_h = ui.fonts_mut(|f| f.row_height(&body_font));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
    let painter = ui.painter_at(rect);

    let num = ui.fonts_mut(|f| f.layout_no_wrap(number.to_string(), num_font, colors.accent));
    painter.galley(
        egui::pos2(rect.min.x, rect.center().y - num.size().y / 2.0),
        num.clone(),
        colors.accent,
    );
    let mut right = rect.max.x;
    if opt.recommended {
        let tag = ui.fonts_mut(|f| {
            f.layout_no_wrap("★ Recommended".to_string(), tag_font, colors.zoom_needs)
        });
        right -= tag.size().x;
        painter.galley(
            egui::pos2(right, rect.center().y - tag.size().y / 2.0),
            tag,
            colors.zoom_needs,
        );
        right -= style::SPACE_SM;
    }
    let label_left = rect.min.x + num.size().x + style::SPACE_XS;
    let label = crate::ui::text::elided_galley(
        ui,
        &opt.label,
        body_font,
        (right - label_left).max(0.0),
        crate::ui::text::Side::Trailing,
    );
    painter.galley(
        egui::pos2(label_left, rect.center().y - label.size().y / 2.0),
        label,
        colors.text_primary,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_send_always_option_names_caller_breadth_and_expiry() {
        let options = pane_send_ask_options();
        assert_eq!(options[1].key, "allow_always");
        assert_eq!(
            options[1].label,
            "Allow this workspace caller any text to any pane for 30 days"
        );
    }
    use crate::broker::gate::{NeedsYouFile, NeedsYouKind, PermissionMonitor};
    use crate::testing::HostHarness;

    // ── Stint 1174: synthetic ingress can never decide a broker request ─────

    use crate::workspace::secrets::{
        InMemoryKeychain, ReadResult, SecretActor, SecretScope, SecretStore,
    };

    const PROTECTED_VALUE: &str = "protected-value";

    /// A real broker pending request (`secret.get` from a pane) with its
    /// Decision Overlay open. "Execution" is the protected read: it returns
    /// the value only after a genuine grant.
    struct BrokerDecision {
        h: HostHarness,
        pane_id: u64,
        monitor: std::sync::Arc<PermissionMonitor>,
        store: InMemoryKeychain,
        actor: SecretActor,
        scope: SecretScope,
        root: tempfile::TempDir,
        pending_id: String,
    }

    impl BrokerDecision {
        fn new() -> Self {
            let mut h = HostHarness::new();
            let pane_id = h.add_focused_terminal();
            let root = tempfile::tempdir().unwrap();
            let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
            let store = InMemoryKeychain::new();
            store.set("plexi:ws:CANARY", PROTECTED_VALUE).unwrap();
            let actor = SecretActor {
                kind: crate::broker::ActorType::Agent,
                id: format!("pane:{pane_id}"),
            };
            let scope = SecretScope::Workspace {
                id: "ws".into(),
                root: root.path().to_owned(),
            };
            let first = crate::workspace::secrets::read_pane_secret(
                &monitor,
                &store,
                &actor,
                "CANARY",
                &scope,
                root.path(),
            )
            .unwrap();
            let ReadResult::PermissionRequired { pending_id, .. } = first else {
                panic!("expected a pending approval, got {first:?}");
            };
            h.app.sync_attention_queue();
            assert!(h.app.open_decision_overlay_for_ask(&pending_id));
            h.run_frames(2);
            let overlay = h.app.attention_queue.decision_overlay.as_ref().unwrap();
            assert!(
                matches!(&overlay.source, QueueSource::BrokerPending { request_id } if *request_id == pending_id),
                "overlay is not bound to the broker pending request"
            );
            Self {
                h,
                pane_id,
                monitor,
                store,
                actor,
                scope,
                root,
                pending_id,
            }
        }

        /// The protected operation, retried exactly as the requester would.
        fn retry_read(&self) -> ReadResult {
            crate::workspace::secrets::read_pane_secret(
                &self.monitor,
                &self.store,
                &self.actor,
                "CANARY",
                &self.scope,
                self.root.path(),
            )
            .unwrap()
        }

        /// Zero decision, zero grant, zero execution, overlay still open.
        fn assert_undecided(&self, origin: &str) {
            assert!(
                self.monitor.show_pending(&self.pending_id).is_some(),
                "{origin}: pending request was resolved"
            );
            assert!(
                self.monitor.approval_resolution(&self.pending_id).is_none(),
                "{origin}: a decision was recorded"
            );
            assert!(
                !self
                    .monitor
                    .list_entries()
                    .iter()
                    .any(|e| e.kind == "allow" || e.kind == "deny"),
                "{origin}: a grant or denial was minted"
            );
            assert!(
                self.h.app.attention_queue.verdicts.is_empty(),
                "{origin}: host verdict recorded"
            );
            assert!(
                self.h.app.attention_queue.answered_ask_ids.is_empty(),
                "{origin}: ask marked answered"
            );
            assert!(
                self.h.app.attention_queue.decision_overlay.is_some(),
                "{origin}: overlay closed"
            );
            assert!(
                matches!(self.retry_read(), ReadResult::PermissionRequired { .. }),
                "{origin}: the protected operation executed or changed state"
            );
        }

        /// Queue one batch exactly as `plexi pane send` / `pane key` / `pane click`
        /// do, run a frame through `raw_input_hook`, and prove the synthetic
        /// marker was delivered with the frame (no vacuous pass).
        fn inject_queued(&mut self, events: Vec<egui::Event>, origin: &str) {
            // Production routes call `pane_navigate` before queueing.
            assert!(
                self.h.app.pane_navigate(self.pane_id),
                "{origin}: pane could not be focused"
            );
            self.h
                .app
                .pending_pane_inputs
                .entry(self.pane_id)
                .or_default()
                .push(egui::RawInput {
                    events,
                    ..Default::default()
                });
            self.h.run_frames(1);
            assert!(
                !self.h.app.pending_pane_inputs.contains_key(&self.pane_id),
                "{origin}: batch not delivered"
            );
            assert!(
                self.h.app.synthetic_input_frame,
                "{origin}: delivered frame was not marked synthetic"
            );
            self.assert_undecided(origin);
        }

        fn option_button_center(&self, option_idx: usize) -> egui::Pos2 {
            let button = self
                .h
                .app
                .approval_buttons
                .iter()
                .filter(|b| b.pending_request_id == self.pending_id)
                .nth(option_idx)
                .expect("decision overlay option button");
            egui::pos2(
                (button.bounds[0] + button.bounds[2]) / 2.0,
                (button.bounds[1] + button.bounds[3]) / 2.0,
            )
        }

        fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
            let button = egui::PointerButton::Primary;
            let modifiers = egui::Modifiers::NONE;
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                },
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    modifiers,
                },
            ]
        }

        fn press_event(key: egui::Key) -> egui::Event {
            Self::key_edge(key, true)
        }

        fn key_edge(key: egui::Key, pressed: bool) -> egui::Event {
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }
        }

        /// A keystroke as the OS delivers it: press then release.
        fn tap(key: egui::Key) -> Vec<egui::Event> {
            vec![Self::key_edge(key, true), Self::key_edge(key, false)]
        }

        /// Desktop frame releasing every decision key, so stages never leak held keys.
        fn release_keys(&mut self) {
            self.h.frame(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                events: [
                    egui::Key::Num1,
                    egui::Key::Num2,
                    egui::Key::Num3,
                    egui::Key::Enter,
                ]
                .into_iter()
                .map(|key| Self::key_edge(key, false))
                .collect(),
                ..Default::default()
            });
        }

        pub const BARRAGE_STAGES: usize = 8;

        /// Every production synthetic ingress, aimed at `option_idx` / its digit key.
        fn synthetic_barrage(&mut self, option_idx: usize, digit: egui::Key) {
            for stage in 0..Self::BARRAGE_STAGES {
                self.barrage_stage(stage, option_idx, digit);
            }
        }

        fn barrage_stage(&mut self, stage: usize, option_idx: usize, digit: egui::Key) {
            let center = self.option_button_center(option_idx);
            match stage {
                0 => {
                    // pane key (view pane passthrough): queued Key batches. Text batches from
                    // pane send carry no Key event and are held until a text surface has focus.
                    self.inject_queued(Self::tap(digit), "pane key digit");
                    self.inject_queued(Self::tap(egui::Key::Enter), "pane key enter");
                }
                1 => {
                    // pane click on a builtin pane: queued pointer click on the button.
                    self.inject_queued(Self::click_events(center), "pane click");
                }
                2 => {
                    // pane drag pacing: one pointer batch per frame, press then release.
                    let button = egui::PointerButton::Primary;
                    let modifiers = egui::Modifiers::NONE;
                    let frames = std::collections::VecDeque::from([
                        egui::RawInput {
                            events: vec![
                                egui::Event::PointerMoved(center),
                                egui::Event::PointerButton {
                                    pos: center,
                                    button,
                                    pressed: true,
                                    modifiers,
                                },
                            ],
                            ..Default::default()
                        },
                        egui::RawInput {
                            events: vec![egui::Event::PointerButton {
                                pos: center,
                                button,
                                pressed: false,
                                modifiers,
                            }],
                            ..Default::default()
                        },
                    ]);
                    self.h
                        .app
                        .pending_pane_pointer_frames
                        .insert(self.pane_id, frames);
                    for _ in 0..2 {
                        // A pointer press can move focus to the pane under it; production
                        // re-focuses per request, so do the same before each paced frame.
                        assert!(
                            self.h.app.pane_navigate(self.pane_id),
                            "pane drag: pane could not be focused"
                        );
                        self.h.run_frames(1);
                        assert!(
                            self.h.app.synthetic_input_frame,
                            "pane drag frame not marked synthetic"
                        );
                        self.assert_undecided("pane drag");
                    }
                    assert!(
                        !self
                            .h
                            .app
                            .pending_pane_pointer_frames
                            .contains_key(&self.pane_id),
                        "drag schedule not drained"
                    );
                }
                3 => {
                    // host key IPC (enters after raw_input_hook, inside `logic`).
                    for key in [digit_str(option_idx), "enter".to_string()] {
                        let response = self.root.path().join(format!("host-key-{key}.json"));
                        self.h.inject_ipc(crate::protocol::HostRequest::HostKey {
                            key: key.clone(),
                            response_file: response.to_string_lossy().into_owned(),
                        });
                        self.h.run_frames(1);
                        let body = std::fs::read_to_string(&response)
                            .expect("host-key request was not handled");
                        assert!(
                            body.contains("\"ok\":true"),
                            "host-key {key} not accepted: {body}"
                        );
                        assert!(
                            self.h.app.synthetic_input_frame,
                            "host-key {key} frame not marked synthetic"
                        );
                        self.assert_undecided(&format!("host key {key}"));
                    }
                }
                4 => {
                    // pane key IPC to the terminal pane writes PTY bytes; it must not reach the overlay.
                    let response = self.root.path().join("key-pane.json");
                    self.h.inject_ipc(crate::protocol::HostRequest::KeyPane {
                        pane_id: self.pane_id,
                        key: digit_str(option_idx),
                        response_file: Some(response.to_string_lossy().into_owned()),
                    });
                    self.h.run_frames(1);
                    assert!(response.exists(), "key-pane request was not handled");
                    self.assert_undecided("key-pane ipc");
                }
                5 => {
                    // Mixed frame: a physical key beside injected input fails closed.
                    self.h.inject_ipc(crate::protocol::HostRequest::HostKey {
                        key: digit_str(option_idx),
                        response_file: self
                            .root
                            .path()
                            .join("mixed.json")
                            .to_string_lossy()
                            .into_owned(),
                    });
                    self.h.frame(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        events: Self::tap(digit),
                        ..Default::default()
                    });
                    assert!(self.h.app.synthetic_input_frame);
                    self.assert_undecided("mixed synthetic and physical frame");
                }
                6 => {
                    // egui rewrites `repeat` to `!first_press`: a key already down
                    // from injected input is a held key when a desktop press arrives.
                    self.inject_queued(vec![Self::press_event(digit)], "injected press left held");
                    self.h.press_key(digit, egui::Modifiers::NONE);
                    assert!(
                        !self.h.app.synthetic_input_frame,
                        "desktop frame was marked synthetic"
                    );
                    self.assert_undecided("desktop press of a key held by injected input");
                }
                7 => {
                    // The decision entry point itself refuses while the frame is marked.
                    self.h.app.synthetic_input_frame = true;
                    self.h.app.request_decision_overlay_option(option_idx);
                    self.h.app.synthetic_input_frame = false;
                    self.assert_undecided("direct decision request");
                }
                _ => unreachable!("barrage stage"),
            }
            self.release_keys();
            self.assert_undecided("after release");
        }

        fn physical_key(&mut self, digit: egui::Key) {
            self.h.press_key(digit, egui::Modifiers::NONE);
            assert!(
                !self.h.app.synthetic_input_frame,
                "physical frame was marked synthetic"
            );
        }

        fn physical_click(&mut self, option_idx: usize) {
            let center = self.option_button_center(option_idx);
            self.h.frame(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                events: Self::click_events(center),
                ..Default::default()
            });
            assert!(
                !self.h.app.synthetic_input_frame,
                "physical click frame was marked synthetic"
            );
        }

        fn assert_decided(&self, choice: crate::broker::gate::ApprovalChoice) {
            use crate::broker::gate::ApprovalChoice;
            assert!(
                self.monitor.show_pending(&self.pending_id).is_none(),
                "genuine decision left the request pending"
            );
            assert_eq!(
                self.monitor.approval_resolution(&self.pending_id),
                Some(choice)
            );
            assert!(
                self.h.app.attention_queue.decision_overlay.is_none(),
                "overlay stayed open after decision"
            );
            assert_eq!(
                self.h.app.attention_queue.verdicts.len(),
                1,
                "exactly one verdict"
            );
            let entry_kind = self
                .monitor
                .list_entries()
                .iter()
                .map(|e| e.kind.clone())
                .collect::<Vec<_>>();
            match choice {
                ApprovalChoice::Deny | ApprovalChoice::DenyAlways => {
                    assert!(
                        !matches!(self.retry_read(), ReadResult::Value(_)),
                        "denied request executed"
                    );
                }
                ApprovalChoice::Once
                | ApprovalChoice::Always
                | ApprovalChoice::Session
                | ApprovalChoice::AllowTool => {
                    assert!(
                        matches!(self.retry_read(), ReadResult::Value(ref v) if v.as_str() == PROTECTED_VALUE),
                        "genuine allow did not permit the operation (entries: {entry_kind:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn seeded_broker_decision_overlay_renders_for_visual_review() {
        let mut f = BrokerDecision::new();
        f.h.run_frames(3);
        assert_eq!(
            f.h.app.approval_buttons.len(),
            3,
            "overlay shows the three decision rows"
        );
        let path = std::env::temp_dir().join("plexi-1174-decision-overlay.png");
        f.h.render_png(&path)
            .expect("render seeded decision overlay");
        assert!(path.exists(), "overlay screenshot was not written");
        log::info!(
            "decision_overlay: seeded screenshot written to {}",
            path.display()
        );
        eprintln!("SCREENSHOT {}", path.display());
    }

    fn digit_str(option_idx: usize) -> String {
        (option_idx + 1).to_string()
    }

    fn assert_option(f: &BrokerDecision, option_idx: usize, key: &str) {
        let overlay = f.h.app.attention_queue.decision_overlay.as_ref().unwrap();
        assert_eq!(
            overlay.options[option_idx].key, key,
            "option order changed; digits would target the wrong choice"
        );
    }

    #[test]
    fn synthetic_ingress_cannot_allow_once_but_physical_key_can() {
        let mut f = BrokerDecision::new();
        assert_option(&f, 0, "allow_once");
        f.synthetic_barrage(0, egui::Key::Num1);
        f.physical_key(egui::Key::Num1);
        f.assert_decided(crate::broker::gate::ApprovalChoice::Once);
    }

    #[test]
    fn synthetic_ingress_cannot_grant_always_but_physical_key_can() {
        let mut f = BrokerDecision::new();
        assert_option(&f, 1, "allow_always");
        f.synthetic_barrage(1, egui::Key::Num2);
        f.physical_key(egui::Key::Num2);
        f.assert_decided(crate::broker::gate::ApprovalChoice::Always);
    }

    #[test]
    fn synthetic_ingress_cannot_deny_but_physical_key_can() {
        let mut f = BrokerDecision::new();
        assert_option(&f, 2, "deny");
        f.synthetic_barrage(2, egui::Key::Num3);
        f.physical_key(egui::Key::Num3);
        f.assert_decided(crate::broker::gate::ApprovalChoice::Deny);
    }

    #[test]
    fn physical_enter_confirms_selected_option_after_synthetic_attempts() {
        let mut f = BrokerDecision::new();
        f.synthetic_barrage(0, egui::Key::Num1);
        f.physical_key(egui::Key::Enter);
        f.assert_decided(crate::broker::gate::ApprovalChoice::Once);
    }

    #[test]
    fn synthetic_ingress_cannot_decide_but_physical_click_can_for_every_option() {
        use crate::broker::gate::ApprovalChoice;
        for (idx, key, digit, choice) in [
            (0, "allow_once", egui::Key::Num1, ApprovalChoice::Once),
            (1, "allow_always", egui::Key::Num2, ApprovalChoice::Always),
            (2, "deny", egui::Key::Num3, ApprovalChoice::Deny),
        ] {
            let mut f = BrokerDecision::new();
            assert_option(&f, idx, key);
            f.synthetic_barrage(idx, digit);
            f.physical_click(idx);
            f.assert_decided(choice);
        }
    }

    fn report_attention(h: &mut HostHarness, pane_id: u64) {
        h.inject_ipc(crate::protocol::HostRequest::SetAgentState {
            peer_ancestry: None,
            pane_id,
            state: crate::protocol::AgentState::NeedsYou,
            agent: "test-agent".into(),
            detail: Some("Which branch should I use?".into()),
            session_id: None,
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();
        h.app.sync_attention_queue();
    }

    fn report_failure_state(h: &mut HostHarness, pane_id: u64, state: crate::protocol::AgentState) {
        h.inject_ipc(crate::protocol::HostRequest::SetAgentState {
            peer_ancestry: None,
            pane_id,
            state,
            agent: "test-agent".into(),
            detail: None,
            session_id: None,
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();
        h.app.sync_attention_queue();
    }

    #[test]
    fn failed_activation_dismisses_and_rearms_after_recovery() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        report_failure_state(&mut h, pane_id, crate::protocol::AgentState::Failed);
        let id = format!("agent-failed-{pane_id}");
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.id == id)
            .unwrap()
            .clone();
        let count = h.app.attention_queue.items.len();
        assert_eq!(
            h.app.activate_attention_bar(&bar),
            AttentionDestination::SourcePane(pane_id)
        );
        assert!(!h.app.attention_queue.items.iter().any(|b| b.id == id));
        assert_eq!(h.app.attention_queue.items.len(), count - 1);
        report_failure_state(&mut h, pane_id, crate::protocol::AgentState::Failed);
        assert!(!h.app.attention_queue.items.iter().any(|b| b.id == id));
        report_failure_state(&mut h, pane_id, crate::protocol::AgentState::Working);
        report_failure_state(&mut h, pane_id, crate::protocol::AgentState::Failed);
        assert!(h.app.attention_queue.items.iter().any(|b| b.id == id));
    }

    #[test]
    fn failed_bar_rearms_when_reports_arrive_before_queue_sync() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        report_failure_state(&mut h, pane_id, crate::protocol::AgentState::Failed);
        let id = format!("agent-failed-{pane_id}");
        assert!(h.app.dismiss_queue_bar(&id));
        for state in [
            crate::protocol::AgentState::Working,
            crate::protocol::AgentState::Failed,
        ] {
            h.inject_ipc(crate::protocol::HostRequest::SetAgentState {
                peer_ancestry: None,
                pane_id,
                state,
                agent: "test-agent".into(),
                detail: None,
                session_id: None,
                event: None,
                blocked_reason: None,
            });
        }
        h.hidden_frame();
        h.app.sync_attention_queue();
        assert!(h.app.attention_queue.items.iter().any(|b| b.id == id));
    }

    #[test]
    fn observed_agent_ask_only_navigates_to_its_pane() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        report_attention(&mut h, pane_id);
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.pane_id == pane_id && bar.category == QueueCategory::Ask)
            .expect("observed agent request must remain visible")
            .clone();
        assert!(
            bar.options.is_empty(),
            "an observed prompt cannot receive a Plexi verdict"
        );
        assert_eq!(
            h.app.resolve_attention_destination(&bar),
            AttentionDestination::SourcePane(pane_id)
        );
        let mut prior = bar.clone();
        prior.options = default_ask_options();
        h.app.attention_queue.decision_overlay = Some(DecisionOverlayState::from_bar(&prior));
        assert!(h.app.open_decision_overlay_for_ask(&bar.id));
        assert!(
            h.app.attention_queue.decision_overlay.is_none(),
            "navigation must close the previous decision"
        );
        assert!(h.app.attention_queue.verdicts.is_empty());
    }

    #[test]
    fn blocked_run_without_choices_opens_pane_without_resolving_record() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        h.inject_ipc(crate::protocol::HostRequest::SetAgentState {
            peer_ancestry: None,
            pane_id,
            state: crate::protocol::AgentState::Blocked,
            agent: "codex".into(),
            detail: Some("Waiting for approval".into()),
            session_id: None,
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();
        h.app.sync_attention_queue();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        let item = monitor
            .open_needs_you()
            .into_iter()
            .find(|item| item.kind == NeedsYouKind::BlockedRun)
            .unwrap();
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == item.id)
            .unwrap()
            .clone();
        assert_eq!(
            h.app
                .attention_queue
                .items
                .iter()
                .filter(|bar| bar.pane_id == pane_id)
                .count(),
            1
        );
        assert!(bar.options.is_empty());
        assert_eq!(
            h.app.activate_attention_bar(&bar),
            AttentionDestination::SourcePane(pane_id)
        );
        assert!(monitor
            .get_needs_you(&item.id)
            .unwrap()
            .resolution
            .is_none());
        assert!(h.app.attention_queue.verdicts.is_empty());
    }

    #[test]
    fn distinct_agent_requests_in_same_pane_are_not_coalesced() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        report_attention(&mut h, pane_id);
        h.inject_ipc(crate::protocol::HostRequest::SetAgentState {
            peer_ancestry: None,
            pane_id,
            state: crate::protocol::AgentState::NeedsYou,
            agent: "test-agent".into(),
            detail: Some("Which test should I run?".into()),
            session_id: None,
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();
        h.app.sync_attention_queue();
        let asks: Vec<_> = h
            .app
            .attention_queue
            .items
            .iter()
            .filter(|bar| bar.pane_id == pane_id && bar.category == QueueCategory::Ask)
            .collect();
        assert_eq!(asks.len(), 2, "distinct requests must remain visible");
        assert_ne!(asks[0].id, asks[1].id);
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        for ask in &asks {
            assert_eq!(
                monitor
                    .get_continuation(&ask.id)
                    .and_then(|c| c.needs_you_id),
                Some(ask.id.clone()),
                "each request must keep its own continuation"
            );
        }
        monitor
            .resolve_needs_you_from(&asks[0].id, false, ResolveSurface::Desktop)
            .unwrap();
        assert!(monitor
            .get_needs_you(&asks[1].id)
            .unwrap()
            .resolution
            .is_none());
        assert_eq!(
            monitor.get_continuation(&asks[1].id).unwrap().state,
            crate::broker::gate::ContinuationState::WaitingUser
        );
    }

    #[test]
    fn permission_with_unknown_outcome_does_not_report_success() {
        let mut h = HostHarness::new();
        h.add_focused_terminal();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        let id = monitor
            .file_needs_you_with_details(
                NeedsYouFile {
                    kind: NeedsYouKind::PermissionRequest,
                    actor: "test-agent".into(),
                    resource: "test-tool".into(),
                    summary: "Allow test tool?".into(),
                    expires_at: None,
                    run_tag: None,
                },
                None,
            )
            .unwrap()
            .0;
        // No workspace or run is available to receive the grant.
        h.app.sync_attention_queue();
        assert!(h.app.open_decision_overlay_for_ask(&id));
        assert!(!h.app.pick_decision_overlay_option(0));
        assert!(h.app.attention_queue.verdicts.is_empty());
        assert!(h
            .app
            .approval_keyboard
            .last_error
            .as_ref()
            .unwrap()
            .1
            .contains("unknown"));
    }

    #[test]
    fn ended_notification_source_has_no_action_choices() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "ended".into(),
                sender_pane_id: pane_id,
                title: "Approval".into(),
                tombstoned: true,
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        let item = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|item| item.id == "notify-ended")
            .unwrap();
        assert!(
            item.options.is_empty(),
            "ended sources cannot receive actions"
        );
    }

    #[test]
    fn failed_notification_response_is_not_reported_as_delivered() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "broken-response".into(),
                sender_pane_id: pane_id,
                title: "Choice".into(),
                response_file: Some("/dev/null/not-a-response".into()),
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h
            .app
            .open_decision_overlay_for_ask("notify-broken-response"));
        assert!(!h.app.pick_decision_overlay_option(0));
        assert!(h.app.attention_queue.verdicts.is_empty());
        assert!(h.app.approval_keyboard.last_error.is_some());
    }

    #[test]
    fn notification_without_live_receiver_is_not_delivered() {
        let mut h = HostHarness::new();
        let terminal = h.add_focused_terminal();
        for pane_id in [terminal, u64::MAX] {
            assert_eq!(
                h.app.deliver_notify_action(
                    pane_id,
                    "dead".into(),
                    "Continue".into(),
                    Some("go".into()),
                    None,
                    None
                ),
                crate::app::notifications::NotifyDelivery::Unavailable,
                "a terminal or missing pane cannot receive a plugin event"
            );
        }
    }

    #[test]
    fn builtin_without_notify_event_handler_is_not_a_receiver() {
        let mut h = HostHarness::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "note").unwrap();
        h.app.open_builtin_view_pane(
            Box::new(crate::app::text_editor_view::TextEditorView::new_for_test_note(path)),
            crate::app::permissions::PluginPermissions::builtin(),
            dir.path().to_path_buf(),
            None,
            Some("split_h"),
            None,
        );
        let pane_id = h.state().open_panes[0];
        assert_eq!(
            h.app.deliver_notify_action(
                pane_id,
                "builtin".into(),
                "Continue".into(),
                None,
                None,
                None
            ),
            crate::app::notifications::NotifyDelivery::Unavailable
        );
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "builtin".into(),
                sender_pane_id: pane_id,
                title: "Choice".into(),
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == "notify-builtin")
            .unwrap()
            .options
            .is_empty());
    }

    #[test]
    fn notification_projection_keeps_only_supported_host_action_and_its_identity() {
        let mut h = HostHarness::new();
        let source = h.add_focused_terminal();
        let destination = h.add_test_pane();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "mixed".into(),
                sender_pane_id: source,
                title: "Choice".into(),
                options: vec![
                    crate::protocol::NotifyOption {
                        label: "Unsupported".into(),
                        value: "bad".into(),
                        shortcut: None,
                        host_action: None,
                    },
                    crate::protocol::NotifyOption {
                        label: "Open pane".into(),
                        value: "go".into(),
                        shortcut: None,
                        host_action: Some(format!("pane_focus:{destination}")),
                    },
                ],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == "notify-mixed")
            .unwrap();
        assert_eq!(bar.options.len(), 1);
        assert_eq!(bar.options[0].label, "Open pane");
        assert!(h.app.open_decision_overlay_for_ask("notify-mixed"));
        assert!(h.app.pick_decision_overlay_option(0));
        assert_eq!(h.app.active_focused_pane_id(), Some(destination));
    }

    #[test]
    fn notification_partial_delivery_cannot_repeat_host_action() {
        let mut h = HostHarness::new();
        let source = h.add_focused_terminal();
        let destination = h.add_test_pane();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "partial".into(),
                sender_pane_id: source,
                title: "Choice".into(),
                response_file: Some("/dev/null/not-a-response".into()),
                options: vec![crate::protocol::NotifyOption {
                    label: "Go".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: Some(format!("pane_focus:{destination}")),
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h.app.open_decision_overlay_for_ask("notify-partial"));
        assert!(!h.app.pick_decision_overlay_option(0));
        assert_eq!(h.app.active_focused_pane_id(), Some(destination));
        assert!(h.app.pane_navigate(source));
        assert!(!h.app.pick_decision_overlay_option(0));
        assert_eq!(
            h.app.active_focused_pane_id(),
            Some(source),
            "retry must not replay host action"
        );
        assert!(h.app.attention_queue.verdicts.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let snapshot = dir.path().join("notifications.json");
        crate::app::notifications::save_pending_notifications_to(
            &h.app.pending_notifications,
            &snapshot,
        );
        h.app.pending_notifications =
            crate::app::notifications::load_pending_notifications_from(&snapshot);
        h.app.attention_queue = AttentionQueueState::default();
        h.app.sync_attention_queue();
        let restored = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == "notify-partial")
            .unwrap();
        assert!(
            restored.summary.contains("Outcome unknown"),
            "restart must retain uncertain outcome"
        );
        assert!(
            restored.options.is_empty(),
            "restart must not expose replayable actions"
        );
        assert_eq!(h.app.active_focused_pane_id(), Some(source));
    }

    #[test]
    fn notification_does_not_execute_when_attempt_cannot_be_persisted() {
        let mut h = HostHarness::new();
        let source = h.add_focused_terminal();
        let destination = h.add_test_pane();
        let blocked_path = crate::config::config_dir().join("notifications.json");
        std::fs::create_dir_all(&blocked_path).unwrap();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "persist-failure".into(),
                sender_pane_id: source,
                title: "Choice".into(),
                options: vec![crate::protocol::NotifyOption {
                    label: "Go".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: Some(format!("pane_focus:{destination}")),
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h
            .app
            .open_decision_overlay_for_ask("notify-persist-failure"));
        assert!(!h.app.pick_decision_overlay_option(0));
        assert_eq!(h.app.active_focused_pane_id(), Some(source));
        assert!(h.app.attention_queue.verdicts.is_empty());
        assert!(h
            .app
            .approval_keyboard
            .last_error
            .as_ref()
            .unwrap()
            .1
            .contains("no action was sent"));
    }

    #[test]
    fn notification_decision_is_serviced_in_hidden_logic() {
        let mut h = HostHarness::new();
        let source = h.add_focused_terminal();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("response");
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "deferred".into(),
                sender_pane_id: source,
                title: "Choice".into(),
                response_file: Some(path.to_string_lossy().into_owned()),
                options: vec![crate::protocol::NotifyOption {
                    label: "Go".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h.app.open_decision_overlay_for_ask("notify-deferred"));
        let mut input = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(matches!(
            h.app.handle_decision_overlay_input(&mut input),
            KeyDisposition::Consumed
        ));
        assert!(!path.exists(), "UI input must not perform external I/O");
        h.hidden_frame();
        assert!(
            path.exists(),
            "pending={:?} error={:?} overlay={:?}",
            h.app.attention_queue.pending_notification_decision,
            h.app.approval_keyboard.last_error,
            h.app
                .attention_queue
                .decision_overlay
                .as_ref()
                .map(|o| &o.ask_id)
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "go");
    }

    #[test]
    fn unbound_notifications_do_not_inherit_or_navigate_to_focused_pane() {
        let mut h = HostHarness::new();
        let focused = h.add_focused_terminal();
        if let Some(pane) = h.app.windows[0]
            .panes
            .get_mut(&focused)
            .and_then(|p| p.as_terminal_mut())
        {
            pane.name = Some("unrelated-codex".into());
        }
        let context = h.app.router.active().context_id;
        let dir = tempfile::tempdir().unwrap();
        let response = dir.path().join("response");
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "unbound-choice".into(),
                title: "CLI request".into(),
                source_context_id: context,
                response_file: Some(response.to_string_lossy().into_owned()),
                required: true,
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "restored".into(),
                title: "Old CLI request".into(),
                source_context_id: context,
                tombstoned: true,
                delivery_unknown: true,
                required: true,
                ..Default::default()
            });
        h.app.sync_attention_queue();
        for id in ["notify-unbound-choice", "notify-restored"] {
            let bar = h
                .app
                .attention_queue
                .items
                .iter()
                .find(|bar| bar.id == id)
                .unwrap();
            assert_eq!(bar.context_id, context);
            assert_eq!(bar.pane_id, 0, "{id} has no pane binding");
            assert_ne!(bar.pane_name, "unrelated-codex");
        }
        assert!(h.app.open_decision_overlay_for_ask("notify-unbound-choice"));
        assert!(h.app.pick_decision_overlay_option(0));
        assert_eq!(std::fs::read_to_string(response).unwrap(), "go");
        assert_eq!(h.app.active_focused_pane_id(), Some(focused));
    }

    #[test]
    fn unbound_question_does_not_inherit_focused_pane() {
        let mut h = HostHarness::new();
        let focused = h.add_focused_terminal();
        let context = h.app.router.active().context_id;
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        let id = monitor
            .file_needs_you_with_details(
                NeedsYouFile {
                    kind: NeedsYouKind::Question,
                    actor: "agent:unbound".into(),
                    resource: "task".into(),
                    summary: "Choose a branch".into(),
                    expires_at: None,
                    run_tag: None,
                },
                Some(crate::broker::gate::NeedsYouDetails {
                    context_id: Some(context),
                    choices: vec!["Alpha".into()],
                    ..Default::default()
                }),
            )
            .unwrap()
            .0;
        h.app.sync_attention_queue();
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == id)
            .unwrap();
        assert_eq!(bar.context_id, context);
        assert_eq!(bar.pane_id, 0);
        assert!(h.app.open_decision_overlay_for_ask(&id));
        assert!(h.app.pick_decision_overlay_option(0));
        assert_eq!(
            monitor.get_needs_you(&id).unwrap().answer.as_deref(),
            Some("Alpha")
        );
        assert_eq!(h.app.active_focused_pane_id(), Some(focused));
    }

    #[test]
    fn notification_response_file_and_host_action_are_valid_routes() {
        let mut h = HostHarness::new();
        let source = h.add_focused_terminal();
        let destination = h.add_test_pane();
        let dir = tempfile::tempdir().unwrap();
        let response = dir.path().join("response");
        assert_eq!(
            h.app.deliver_notify_action(
                u64::MAX,
                "file".into(),
                "Continue".into(),
                Some("go".into()),
                Some(response.to_string_lossy().into_owned()),
                None
            ),
            crate::app::notifications::NotifyDelivery::Delivered
        );
        assert_eq!(std::fs::read_to_string(response).unwrap(), "go");
        assert_eq!(
            h.app.deliver_notify_action(
                source,
                "host".into(),
                "Open".into(),
                None,
                None,
                Some(format!("pane_focus:{destination}"))
            ),
            crate::app::notifications::NotifyDelivery::Delivered
        );
        assert_eq!(h.app.active_focused_pane_id(), Some(destination));
    }

    #[test]
    fn dead_sender_does_not_hide_live_response_file_route() {
        let mut h = HostHarness::new();
        h.add_focused_terminal();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("response");
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "independent-file".into(),
                sender_pane_id: u64::MAX,
                title: "Choice".into(),
                response_file: Some(path.to_string_lossy().into_owned()),
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h
            .app
            .open_decision_overlay_for_ask("notify-independent-file"));
        assert!(h.app.pick_decision_overlay_option(0));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "go");
    }

    #[test]
    fn dead_sender_with_unknown_delivery_keeps_error_visible() {
        let mut h = HostHarness::new();
        h.add_focused_terminal();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "unknown-dead".into(),
                sender_pane_id: u64::MAX,
                title: "Choice".into(),
                response_file: Some("/dev/null/not-a-response".into()),
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h.app.open_decision_overlay_for_ask("notify-unknown-dead"));
        assert!(!h.app.pick_decision_overlay_option(0));
        h.app.sync_attention_queue();
        assert!(
            h.app.attention_queue.decision_overlay.is_some(),
            "outcome error must remain visible"
        );
        assert!(h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == "notify-unknown-dead")
            .unwrap()
            .summary
            .contains("Outcome unknown"));
        assert!(!h.app.pick_decision_overlay_option(0));
    }

    #[test]
    fn removed_notification_cannot_receive_a_verdict() {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        h.app
            .pending_notifications
            .push(crate::app::notifications::PendingNotification {
                notify_id: "stale".into(),
                sender_pane_id: pane_id,
                title: "Choice".into(),
                response_file: Some("/dev/null/not-a-response".into()),
                options: vec![crate::protocol::NotifyOption {
                    label: "Continue".into(),
                    value: "go".into(),
                    shortcut: None,
                    host_action: None,
                }],
                ..Default::default()
            });
        h.app.sync_attention_queue();
        assert!(h.app.open_decision_overlay_for_ask("notify-stale"));
        h.app.pending_notifications.clear();
        assert!(!h.app.pick_decision_overlay_option(0));
        assert!(h.app.attention_queue.verdicts.is_empty());
        assert!(h.app.approval_keyboard.last_error.is_some());
    }

    fn needs_you_failed_audit_is_retryable(with_choices: bool) {
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        let mut permission_id = None;
        if with_choices {
            // Rich asks use the answer branch; agent reports reuse the same run tag.
            monitor
                .file_needs_you_with_details(
                    NeedsYouFile {
                        kind: NeedsYouKind::Question,
                        actor: "test-agent".into(),
                        resource: format!("pane:{pane_id}"),
                        summary: "Which branch should I use?".into(),
                        expires_at: None,
                        run_tag: Some(format!("pane:{pane_id}")),
                    },
                    Some(crate::broker::gate::NeedsYouDetails {
                        choices: vec!["Alpha".into(), "Beta".into()],
                        pane_id: Some(pane_id),
                        ..Default::default()
                    }),
                )
                .unwrap();
        } else {
            permission_id = Some(
                monitor
                    .file_needs_you_with_details(
                        NeedsYouFile {
                            kind: NeedsYouKind::PermissionRequest,
                            actor: "test-agent".into(),
                            resource: "test-tool".into(),
                            summary: "Allow test tool?".into(),
                            expires_at: None,
                            run_tag: None,
                        },
                        Some(crate::broker::gate::NeedsYouDetails {
                            pane_id: Some(pane_id),
                            ..Default::default()
                        }),
                    )
                    .unwrap()
                    .0,
            );
        }
        report_attention(&mut h, pane_id);
        let id = permission_id.unwrap_or_else(|| {
            monitor
                .open_needs_you()
                .into_iter()
                .find(|item| item.kind == NeedsYouKind::Question)
                .unwrap()
                .id
        });
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| bar.id == id)
            .unwrap();
        assert!(
            matches!(bar.source, QueueSource::NeedsYou { is_question, .. } if is_question == with_choices)
        );
        assert!(h.app.open_decision_overlay_for_ask(&id));

        monitor.fail_audit(true);
        let accepted = h.app.pick_decision_overlay_option(0);
        // Prove the failure reached audit, rather than a desktop-origin refusal.
        assert!(monitor.get_needs_you(&id).unwrap().resolution.is_none());
        assert!(
            !accepted,
            "failed audit must not report a successful decision"
        );
        assert!(!h.app.attention_queue.answered_ask_ids.contains(&id));
        assert!(h.app.attention_queue.verdicts.is_empty());
        assert_eq!(
            h.app
                .attention_queue
                .decision_overlay
                .as_ref()
                .unwrap()
                .ask_id,
            id
        );
        let (error_id, message) = h.app.approval_keyboard.last_error.as_ref().unwrap();
        assert_eq!(error_id, &id);
        assert!(
            message.contains("audit"),
            "expected audit error, got {message}"
        );

        // Same report must keep the unresolved broker id visible, not just a fallback.
        report_attention(&mut h, pane_id);
        assert!(monitor.open_needs_you().iter().any(|item| item.id == id));
        assert!(h.app.attention_queue.items.iter().any(|bar| bar.id == id));
        monitor.fail_audit(false);
        assert!(h.app.pick_decision_overlay_option(0));
        assert!(monitor.get_needs_you(&id).unwrap().resolution.is_some());
        if with_choices {
            assert_eq!(
                monitor.get_needs_you(&id).unwrap().answer.as_deref(),
                Some("Alpha")
            );
        }
        assert!(h.app.approval_keyboard.last_error.is_none());
        assert!(h.app.attention_queue.decision_overlay.is_none());
        assert_eq!(h.app.attention_queue.verdicts.len(), 1);

        // Agent state remains an observed signal, not a second approval endpoint.
        h.app.sync_attention_queue();
        if let Some(fallback) = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|bar| matches!(bar.source, QueueSource::AgentState { .. }))
        {
            assert!(fallback.options.is_empty());
        }
    }

    #[test]
    fn needs_you_resolve_failed_audit_remains_retryable() {
        needs_you_failed_audit_is_retryable(false);
    }

    #[test]
    fn needs_you_answer_failed_audit_remains_retryable() {
        needs_you_failed_audit_is_retryable(true);
    }

    #[test]
    fn per_context_ask_count_matches_queue_labels_and_sidebar_badge() {
        let mut h = HostHarness::new();
        let ctx_alpha = h.app.router.active().context_id;
        h.app.rename_context(0, "ctx-alpha");
        let a1 = h.add_test_pane();
        let a2 = h.add_test_pane();
        let a3 = h.add_test_pane();
        if let Some(p) = h.app.windows[0]
            .panes
            .get_mut(&a1)
            .and_then(|p| p.as_terminal_mut())
        {
            p.name = Some("a1".to_string());
        }
        if let Some(p) = h.app.windows[0]
            .panes
            .get_mut(&a2)
            .and_then(|p| p.as_terminal_mut())
        {
            p.name = Some("a2".to_string());
        }
        if let Some(p) = h.app.windows[0]
            .panes
            .get_mut(&a3)
            .and_then(|p| p.as_terminal_mut())
        {
            p.name = Some("a3".to_string());
        }

        let ctx_beta = h.app.create_context_without_pty("ctx-beta", None);
        let beta_idx = h.app.router.active_idx();
        let pane_5 = h.add_test_pane();
        h.app.set_context_columns(ctx_beta, vec![vec![pane_5]]);

        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::Question,
                actor: "claude-code".to_string(),
                resource: format!("pane:{a3}"),
                summary: "claude-code is blocked".to_string(),
                expires_at: None,
                run_tag: Some(format!("pane:{a3}")),
            })
            .unwrap();
        monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::Question,
                actor: "codex".to_string(),
                resource: format!("pane:{pane_5}"),
                summary: "codex is blocked".to_string(),
                expires_at: None,
                run_tag: Some(format!("pane:{pane_5}")),
            })
            .unwrap();

        h.app
            .enqueue_ask("ask-a2", a2, "cargo test", 1_000, Vec::new());
        h.app
            .enqueue_ask("ask-pane5-2", pane_5, "git push", 2_000, Vec::new());

        for focus_idx in [beta_idx, 0] {
            h.app.switch_workspace(focus_idx);
            h.app.sync_attention_queue();

            for (cid, label) in [(ctx_alpha, "ctx-alpha"), (ctx_beta, "ctx-beta")] {
                let count = h.app.attention_queue.ask_count_for_context(cid);
                let label_count = h
                    .app
                    .attention_queue
                    .items
                    .iter()
                    .filter(|b| b.category == QueueCategory::Ask && b.context_path == label)
                    .count();
                assert_eq!(count, 2, "expected 2 asks in {label}");
                assert_eq!(
                    count, label_count,
                    "ask_count_for_context({label}) must equal ask bars labeled {label}"
                );
                assert_eq!(
                    h.app.sidebar_context_badge_count(cid),
                    count,
                    "sidebar badge for {label} must use ask_count_for_context"
                );
            }
        }
    }

    #[test]
    fn enter_on_notice_opens_decision_overlay_and_records_acknowledged_resolution() {
        let mut h = HostHarness::new();
        let pane_id = h.add_test_pane();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();

        let notice_id = monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::Integrity,
                actor: "app-update".to_string(),
                resource: format!("pane:{pane_id}"),
                summary: "Plexi updated in the background".to_string(),
                expires_at: None,
                run_tag: Some(format!("pane:{pane_id}")),
            })
            .unwrap();

        h.app.toggle_attention_queue_list();
        assert!(h.app.attention_queue.open);
        let notice_idx = h
            .app
            .attention_queue
            .items
            .iter()
            .position(|b| b.id == notice_id && b.category == QueueCategory::Notice)
            .expect("notice item in attention queue");
        h.app.attention_queue.selected_idx = notice_idx;

        let mut enter_input = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        let disp = h.app.handle_attention_queue_list_input(&mut enter_input);
        assert!(matches!(disp, KeyDisposition::Consumed));

        // Enter on Notice must open Decision Overlay with acknowledge_options() (Acknowledge starred)
        // and NOT immediately acknowledge the notice in the store.
        let overlay = h
            .app
            .attention_queue
            .decision_overlay
            .as_ref()
            .expect("Enter on Notice must open decision overlay");
        assert_eq!(overlay.ask_id, notice_id);
        assert_eq!(overlay.options, acknowledge_options());
        assert!(overlay.options[0].recommended);
        assert_eq!(overlay.options[0].label, "Acknowledge");
        assert!(
            monitor.open_needs_you().iter().any(|r| r.id == notice_id),
            "notice must remain open while Decision Overlay is displayed"
        );

        // Picking Acknowledge in the overlay resolves the notice with resolution=acknowledged.
        let mut pick_input = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        let disp = h.app.handle_decision_overlay_input(&mut pick_input);
        assert!(matches!(disp, KeyDisposition::Consumed));
        assert!(h.app.attention_queue.decision_overlay.is_none());

        let record = monitor
            .get_needs_you(&notice_id)
            .expect("resolved notice record");
        assert_eq!(
            record.resolution,
            Some(crate::broker::gate::NeedsYouResolution::Acknowledged)
        );
        assert_eq!(record.resolution.map(|r| r.as_str()), Some("acknowledged"));

        // Verify auto-clear-once-seen still works when opening & closing the list without Enter.
        let auto_notice_id = monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::Integrity,
                actor: "app-update".to_string(),
                resource: "auto-clear-resource".to_string(),
                summary: "Background maintenance completed".to_string(),
                expires_at: None,
                run_tag: None,
            })
            .unwrap();
        h.app.toggle_attention_queue_list();
        assert!(h.app.attention_queue.open);
        h.app.toggle_attention_queue_list();
        assert!(!h.app.attention_queue.open);
        let auto_record = monitor
            .get_needs_you(&auto_notice_id)
            .expect("auto-cleared notice record");
        assert_eq!(
            auto_record.resolution,
            Some(crate::broker::gate::NeedsYouResolution::Acknowledged)
        );
    }

    fn option_row_heights(recommend_first: bool) -> Vec<f32> {
        let mut h = HostHarness::new();
        let id = "ask-rows";
        let pane = h.add_test_pane();
        h.app.enqueue_ask(id, pane, "cargo test", 1_000, Vec::new());
        assert!(h.app.open_decision_overlay_for_ask(id));
        let overlay = h.app.attention_queue.decision_overlay.as_mut().unwrap();
        overlay.options = default_ask_options();
        overlay.options[0].recommended = recommend_first;
        overlay.options[1].label =
            "A very long option label that cannot possibly fit on a single line of the card, \
             and keeps going well past the modal width"
                .to_string();
        h.run_frames(3);
        h.app
            .approval_buttons
            .iter()
            .map(|b| b.bounds[3] - b.bounds[1])
            .collect()
    }

    #[test]
    fn decision_overlay_option_rows_have_equal_height() {
        for recommend_first in [true, false] {
            let heights = option_row_heights(recommend_first);
            assert_eq!(heights.len(), 3, "three option rows");
            assert!(
                heights.iter().all(|h| (h - heights[0]).abs() < 0.5),
                "rows differ (recommended={recommend_first}): {heights:?}"
            );
        }
        let with = option_row_heights(true);
        let without = option_row_heights(false);
        assert!(
            (with[0] - without[0]).abs() < 0.5,
            "tag must not inflate the row"
        );
    }

    #[test]
    fn attention_queue_header_never_duplicate_counts() {
        // total == 0
        let h_zero = format_attention_queue_header_count(0, 0);
        assert_eq!(h_zero, "0 items");

        // asks == 0, total > 0
        let h_no_asks = format_attention_queue_header_count(0, 3);
        assert_eq!(h_no_asks, "3 items");
        assert!(
            !h_no_asks.contains('0'),
            "asks==0 must not print ◆0: {h_no_asks}"
        );

        // asks == total (must show the number 3 only once!)
        let h_all_asks = format_attention_queue_header_count(3, 3);
        assert_eq!(h_all_asks, "◆3 items");
        assert_eq!(
            h_all_asks.matches('3').count(),
            1,
            "asks==total must print the count once: {h_all_asks}"
        );

        // singular asks == total == 1
        assert_eq!(format_attention_queue_header_count(1, 1), "◆1 item");

        // 0 < asks < total
        let h_mixed = format_attention_queue_header_count(2, 5);
        assert_eq!(h_mixed, "◆2 · 5 items");
    }

    #[test]
    fn attention_mailbox_formatting_states() {
        // Empty state: lowered flag, 0 count, Empty visual state
        let (icon, count, state) = format_attention_mailbox(0, 0, 0);
        assert_eq!(icon, "\u{1F4ED}");
        assert_eq!(count, "0");
        assert_eq!(state, AttentionMailboxVisualState::Empty);

        // Asks present: raised flag, asks count, AsksPresent state
        let (icon, count, state) = format_attention_mailbox(1, 0, 1);
        assert_eq!(icon, "\u{1F4EC}");
        assert_eq!(count, "1");
        assert_eq!(state, AttentionMailboxVisualState::AsksPresent);

        // Asks present with additional non-ask items: asks · total
        let (icon, count, state) = format_attention_mailbox(2, 1, 5);
        assert_eq!(icon, "\u{1F4EC}");
        assert_eq!(count, "2 · 5");
        assert_eq!(state, AttentionMailboxVisualState::AsksPresent);

        // Failure state: 0 asks, failed > 0: raised flag, total count, Failed state
        let (icon, count, state) = format_attention_mailbox(0, 2, 3);
        assert_eq!(icon, "\u{1F4EC}");
        assert_eq!(count, "3");
        assert_eq!(state, AttentionMailboxVisualState::Failed);

        // Result/notice state: 0 asks, 0 failed, total > 0: raised flag, total count, ResultOrNotice state
        let (icon, count, state) = format_attention_mailbox(0, 0, 4);
        assert_eq!(icon, "\u{1F4EC}");
        assert_eq!(count, "4");
        assert_eq!(state, AttentionMailboxVisualState::ResultOrNotice);
    }

    fn add_test_terminal_pane(
        h: &mut HostHarness,
        agent: Option<(&str, &str)>,
        parent_id: Option<u64>,
        spawned_by: Option<&str>,
    ) -> u64 {
        let id = h.app.host.alloc_pane_id();
        let mut term = crate::host::pane::TerminalPane::new(
            id,
            h.app.ctx.clone(),
            h.app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                ..Default::default()
            },
            14.0,
        )
        .expect("create test terminal pane");
        if let Some((harness, role)) = agent {
            term.agent_harness = Some(harness.into());
            term.agent_role = Some(role.into());
            term.agent = Some(crate::protocol::PaneAgentState {
                pane_id: id,
                state: crate::protocol::AgentState::Idle,
                agent: harness.into(),
                detail: None,
                session_id: None,
            });
        }
        term.parent_pane_id = parent_id;
        term.spawned_by = spawned_by.map(str::to_string);
        let win = &mut h.app.windows[0];
        win.panes
            .insert(id, crate::host::pane::Pane::Terminal(Box::new(term)));
        let tile = win.tree.tiles.insert_pane(id);
        if win.tree.root().is_none() {
            win.tree.root = Some(tile);
        }
        id
    }

    #[test]
    fn attention_result_nav_live_parent_with_closed_child() {
        let mut h = HostHarness::new();
        let p_parent = add_test_terminal_pane(&mut h, Some(("claude", "lead")), None, None);
        let p_child = add_test_terminal_pane(
            &mut h,
            Some(("claude", "worker")),
            Some(p_parent),
            Some("agent:lead"),
        );

        // Child records result
        h.app.record_child_agent_result(
            Some(p_parent),
            crate::workspace::QueuedChildResult {
                child_pane_id: p_child,
                child_agent: "worker".into(),
                status: "completed".into(),
                summary: Some("subtask finished".into()),
                result_json: "{}".into(),
                finished_at_epoch_secs: 1000,
            },
        );

        // Child pane is closed
        h.app.close_pane_by_id(p_child);
        assert!(h.app.find_pane_in_any_window(p_child).is_none());

        h.app.sync_attention_queue();
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.pane_id == p_child)
            .expect("child result present in attention queue")
            .clone();

        assert_eq!(bar.pane_name, "worker");
        assert_eq!(bar.summary, "subtask finished");

        // Destination must resolve to live parent
        let dest = h.app.resolve_attention_destination(&bar);
        assert_eq!(dest, AttentionDestination::LiveParent(p_parent));

        // Enter activation jumps to parent
        h.app.toggle_attention_queue_list();
        assert!(h.app.attention_queue.open);
        let idx = h
            .app
            .attention_queue
            .items
            .iter()
            .position(|b| b.id == bar.id)
            .unwrap();
        h.app.attention_queue.selected_idx = idx;

        let mut enter_input = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        let disp = h.app.handle_attention_queue_list_input(&mut enter_input);
        assert!(matches!(disp, KeyDisposition::Consumed));
        assert!(!h.app.attention_queue.open);
        assert_eq!(h.app.active_focused_pane_id(), Some(p_parent));

        // Result remains reachable in the queue
        h.app.sync_attention_queue();
        assert!(
            h.app
                .attention_queue
                .items
                .iter()
                .any(|b| b.pane_id == p_child),
            "child result must remain reachable in the queue"
        );
    }

    #[test]
    fn attention_result_nav_human_started_live_and_closed() {
        let mut h = HostHarness::new();
        let p_child = add_test_terminal_pane(&mut h, Some(("claude", "solo")), None, Some("human"));

        h.app.record_child_agent_result(
            None,
            crate::workspace::QueuedChildResult {
                child_pane_id: p_child,
                child_agent: "solo".into(),
                status: "completed".into(),
                summary: Some("solo done".into()),
                result_json: "{}".into(),
                finished_at_epoch_secs: 1000,
            },
        );

        h.app.sync_attention_queue();
        let bar = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.pane_id == p_child)
            .unwrap()
            .clone();

        // 1. Live child: falls back to SourcePane
        let dest_live = h.app.resolve_attention_destination(&bar);
        assert_eq!(dest_live, AttentionDestination::SourcePane(p_child));

        // 2. Closed child: falls back to ResultArtifact
        h.app.close_pane_by_id(p_child);
        let dest_closed = h.app.resolve_attention_destination(&bar);
        match dest_closed {
            AttentionDestination::ResultArtifact(ref path) => {
                assert!(path.ends_with("RESULT.md"));
                assert!(
                    !path.exists(),
                    "resolving a destination must not create a result artifact"
                );
            }
            other => panic!("expected ResultArtifact, got {other:?}"),
        }

        // Activation opens text-editor viewer pane
        h.app.activate_attention_bar(&bar);
        match dest_closed {
            AttentionDestination::ResultArtifact(ref path) => {
                assert!(
                    path.is_file(),
                    "opening the fallback artifact must synthesize it only on activation"
                );
            }
            _ => unreachable!("checked ResultArtifact above"),
        }
        assert!(
            h.app
                .windows
                .iter()
                .flat_map(|w| w.panes.values())
                .any(|p| matches!(p, crate::host::pane::Pane::View(app) if app.manifest_id == "text-editor")),
            "activating closed human-started result must open text-editor"
        );
    }

    #[test]
    fn attention_result_nav_absent_parent_and_non_agent_parent() {
        let mut h = HostHarness::new();
        // Case A: Absent parent
        let p_parent = add_test_terminal_pane(&mut h, Some(("claude", "lead")), None, None);
        let p_child = add_test_terminal_pane(
            &mut h,
            Some(("claude", "child_a")),
            Some(p_parent),
            Some("agent:lead"),
        );
        h.app.record_child_agent_result(
            Some(p_parent),
            crate::workspace::QueuedChildResult {
                child_pane_id: p_child,
                child_agent: "child_a".into(),
                status: "completed".into(),
                summary: Some("done a".into()),
                result_json: "{}".into(),
                finished_at_epoch_secs: 1000,
            },
        );
        // Both parent and child close
        h.app.close_pane_by_id(p_parent);
        h.app.close_pane_by_id(p_child);
        h.app.sync_attention_queue();
        let bar_a = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.pane_id == p_child)
            .unwrap()
            .clone();
        let dest_a = h.app.resolve_attention_destination(&bar_a);
        assert!(matches!(dest_a, AttentionDestination::ResultArtifact(_)));

        // Case B: Non-agent parent (terminal with no agent harness/role)
        let p_shell = add_test_terminal_pane(&mut h, None, None, None);
        let p_child_b = add_test_terminal_pane(
            &mut h,
            Some(("claude", "child_b")),
            Some(p_shell),
            Some("agent:shell"),
        );
        h.app.record_child_agent_result(
            Some(p_shell),
            crate::workspace::QueuedChildResult {
                child_pane_id: p_child_b,
                child_agent: "child_b".into(),
                status: "completed".into(),
                summary: Some("done b".into()),
                result_json: "{}".into(),
                finished_at_epoch_secs: 1000,
            },
        );
        h.app.close_pane_by_id(p_child_b);
        h.app.sync_attention_queue();
        let bar_b = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.pane_id == p_child_b)
            .unwrap()
            .clone();
        assert_eq!(h.app.find_parent_pane_id(p_child_b), Some(p_shell));
        let (parent_window, _) = h
            .app
            .find_pane_in_any_window(p_shell)
            .expect("live non-agent parent");
        assert!(
            !h.app.windows[parent_window]
                .panes
                .get(&p_shell)
                .expect("live non-agent parent pane")
                .is_agent_pane(),
            "the parent branch must be reached and reject a live non-agent pane"
        );
        let dest_b = h.app.resolve_attention_destination(&bar_b);
        assert!(matches!(dest_b, AttentionDestination::ResultArtifact(_)));
    }

    #[test]
    fn attention_result_nav_enter_preserves_ask_overlay() {
        let mut h = HostHarness::new();
        let p_parent = add_test_terminal_pane(&mut h, Some(("claude", "lead")), None, None);
        let p_child = add_test_terminal_pane(
            &mut h,
            Some(("claude", "child")),
            Some(p_parent),
            Some("agent:lead"),
        );
        h.app.record_child_agent_result(
            Some(p_parent),
            crate::workspace::QueuedChildResult {
                child_pane_id: p_child,
                child_agent: "child".into(),
                status: "completed".into(),
                summary: Some("done".into()),
                result_json: "{}".into(),
                finished_at_epoch_secs: 1000,
            },
        );
        h.app.close_pane_by_id(p_child);

        // Also add an ask
        let p_ask = h.add_test_pane();
        h.app
            .enqueue_ask("ask-test", p_ask, "cargo test", 500, Vec::new());

        h.app.sync_attention_queue();
        let bar_res = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.pane_id == p_child)
            .unwrap()
            .clone();
        let bar_ask = h
            .app
            .attention_queue
            .items
            .iter()
            .find(|b| b.id == "ask-test")
            .unwrap()
            .clone();

        // 1. Enter drives the real queue activation path to the live parent.
        h.app.toggle_attention_queue_list();
        let result_idx = h
            .app
            .attention_queue
            .items
            .iter()
            .position(|bar| bar.id == bar_res.id)
            .expect("result bar");
        h.app.attention_queue.selected_idx = result_idx;
        let mut enter_input = PlexiInput::synthetic(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(matches!(
            h.app.handle_attention_queue_list_input(&mut enter_input),
            KeyDisposition::Consumed
        ));
        assert_eq!(h.app.active_focused_pane_id(), Some(p_parent));

        // 2. Ask overlay semantics stay on the shared activation helper.
        let dest_ask = h.app.resolve_attention_destination(&bar_ask);
        assert_eq!(
            dest_ask,
            AttentionDestination::DecisionOverlay("ask-test".into())
        );
        h.app.activate_attention_bar(&bar_ask);
        assert!(h.app.attention_queue.decision_overlay.is_some());
        assert_eq!(
            h.app
                .attention_queue
                .decision_overlay
                .as_ref()
                .unwrap()
                .ask_id,
            "ask-test"
        );
    }
}

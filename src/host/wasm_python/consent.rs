//! In-pane consent for a Python plugin's sensitive capabilities (stint 1489,
//! decisions ledger row 103).
//!
//! A declared consent capability is decided at each use: the permission
//! monitor's stored row first (so a grant, revoke or reset made while the
//! plugin runs applies to its next request), then the pane's own answer for
//! this pane's lifetime, then a prompt in the pane. The request that asked
//! waits for the answer and is replayed through the normal handler once it is
//! given. "Always" answers are stored through `PermissionMonitor`; nothing
//! here mints an allow without a click.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::app::permissions::Capability;
use crate::broker::gate::PermissionMonitor;
use crate::broker::Decision;

/// The capabilities a Python plugin is asked about in its pane. Every other
/// sensitive capability keeps the launch-set rule until stint 1238 moves all
/// of them to broker admission.
pub(crate) const CONSENT_CAPABILITIES: [Capability; 3] = [
    Capability::NetHttp,
    Capability::FsWrite,
    Capability::AiQuery,
];

/// Requests one pane may hold while its prompt waits. A plugin that keeps
/// asking (a poll timer) cannot grow the queue without bound.
pub(crate) const MAX_PARKED: usize = 32;

pub(crate) fn is_consent_capability(capability: &str) -> bool {
    CONSENT_CAPABILITIES
        .iter()
        .any(|candidate| candidate.as_str() == capability)
}

/// Why a consent capability was refused. Each reason has its own way back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The manifest does not declare it. Only the plugin author can fix this.
    Undeclared,
    /// Not the host process, so there is no permission monitor to ask.
    NotHost,
    /// A stored denial; `entry_id` is the row `plexi permissions allow` takes.
    Stored { entry_id: String },
    /// Denied for this pane's lifetime.
    Pane,
    /// Too many requests already wait for the answer.
    Backlog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsentAdmission {
    Admit { mechanism: &'static str },
    Ask,
    Refuse(Refusal),
}

impl ConsentAdmission {
    pub(crate) fn decision(&self) -> &'static str {
        match self {
            Self::Admit { .. } => "allow",
            Self::Ask => "ask",
            Self::Refuse(_) => "deny",
        }
    }

    pub(crate) fn mechanism(&self) -> &'static str {
        match self {
            Self::Admit { mechanism } => mechanism,
            Self::Ask => "python_consent_ask",
            Self::Refuse(Refusal::Undeclared) => "python_undeclared",
            Self::Refuse(Refusal::NotHost) => "python_not_host",
            Self::Refuse(Refusal::Stored { .. }) => "python_live_row",
            Self::Refuse(Refusal::Pane) => "python_pane_answer",
            Self::Refuse(Refusal::Backlog) => "python_consent_backlog",
        }
    }
}

/// Decide one use of a consent capability. `pane_answer` is this pane's
/// unremembered answer, if it gave one.
pub(crate) fn consent_admission(
    monitor: Option<&PermissionMonitor>,
    plugin_id: &str,
    workspace_root: &Path,
    declared: &[String],
    pane_answer: Option<bool>,
    capability: &str,
) -> ConsentAdmission {
    if !declared.iter().any(|item| item == capability) {
        return ConsentAdmission::Refuse(Refusal::Undeclared);
    }
    let Some(monitor) = monitor else {
        return ConsentAdmission::Refuse(Refusal::NotHost);
    };
    match monitor.live_capability_decision(plugin_id, workspace_root, capability) {
        Some((Decision::Allow, _)) => ConsentAdmission::Admit {
            mechanism: "python_live_row",
        },
        Some((Decision::Deny, entry_id)) => ConsentAdmission::Refuse(Refusal::Stored { entry_id }),
        Some((Decision::Ask, _)) | None => match pane_answer {
            Some(true) => ConsentAdmission::Admit {
                mechanism: "python_pane_answer",
            },
            Some(false) => ConsentAdmission::Refuse(Refusal::Pane),
            None => ConsentAdmission::Ask,
        },
    }
}

/// What the plugin is told when a consent capability is refused. Names the
/// capability and the way to allow it.
pub(crate) fn refusal_text(plugin_id: &str, capability: &str, refusal: &Refusal) -> String {
    match refusal {
        Refusal::Undeclared => format!("missing capability {capability}"),
        Refusal::NotHost => {
            format!("{capability} is unavailable: this process is not the Plexi host")
        }
        Refusal::Stored { entry_id } if !entry_id.is_empty() => format!(
            "{capability} is denied for {plugin_id}. To allow it, use the Permissions plugin \
             or run `plexi permissions allow {entry_id}`."
        ),
        Refusal::Stored { .. } => format!(
            "{capability} is denied for {plugin_id}. To allow it, use the Permissions plugin \
             or `plexi permissions list`."
        ),
        Refusal::Pane => {
            format!("{capability} was denied for this pane. Reopen {plugin_id} to be asked again.")
        }
        Refusal::Backlog => format!(
            "{capability} request dropped: {MAX_PARKED} requests already wait for your answer"
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonCapabilityPrompt {
    /// Fresh per prompt; the consent guard arms on it.
    pub(crate) request_id: u64,
    pub(crate) capability: String,
}

impl PythonCapabilityPrompt {
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }
}

/// One pane's consent state. Lives exactly as long as the pane.
#[derive(Default)]
pub(crate) struct PaneConsent {
    pub(crate) prompts: VecDeque<PythonCapabilityPrompt>,
    /// Requests waiting on a prompt, by capability, in arrival order.
    pub(crate) parked: Vec<(String, Value)>,
    /// Unremembered answers, for this pane's lifetime.
    pub(crate) answers: HashMap<String, bool>,
    /// Requests whose answer arrived, handled again on the next drain.
    pub(crate) replay: VecDeque<Value>,
    /// The profile's monitor, already opened by the launch gate; `None`
    /// outside the host, where every consent capability is refused.
    pub(crate) monitor: Option<Arc<PermissionMonitor>>,
}

impl PaneConsent {
    pub(crate) fn for_profile() -> Self {
        Self {
            monitor: PermissionMonitor::loaded(&crate::config::config_dir()),
            ..Self::default()
        }
    }

    /// Hold `message` until `capability` is answered and show its prompt
    /// once. `false` when the backlog is full and the request was not held.
    pub(crate) fn park(&mut self, capability: &str, message: Value) -> bool {
        if self.parked.len() >= MAX_PARKED {
            return false;
        }
        self.parked.push((capability.to_string(), message));
        if !self
            .prompts
            .iter()
            .any(|prompt| prompt.capability == capability)
        {
            self.prompts.push_back(PythonCapabilityPrompt {
                request_id: crate::app::approval_keyboard::next_consent_request_id(),
                capability: capability.to_string(),
            });
        }
        true
    }

    /// Record the answer to the front prompt and queue its held requests for
    /// replay. Returns the answered capability.
    pub(crate) fn answer_front(&mut self, granted: bool, remember: bool) -> Option<String> {
        let prompt = self.prompts.pop_front()?;
        if remember {
            self.answers.remove(&prompt.capability);
        } else {
            self.answers.insert(prompt.capability.clone(), granted);
        }
        let (due, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.parked)
            .into_iter()
            .partition(|(capability, _)| capability == &prompt.capability);
        self.parked = held;
        self.replay
            .extend(due.into_iter().map(|(_, message)| message));
        Some(prompt.capability)
    }
}

/// The prompt body, one caption per line. The plugin is named by its id, which
/// the host owns; the guest-set title could claim to be anything.
pub(crate) fn prompt_lines(
    plugin_id: &str,
    capability: &str,
    allowed_hosts: &[String],
) -> Vec<String> {
    let mut lines = vec![format!("{plugin_id} requests:")];
    match Capability::try_from(capability) {
        Ok(known) => lines.push(format!("{capability}: {}", known.description())),
        Err(_) => lines.push(capability.to_string()),
    }
    if capability == Capability::NetHttp.as_str() {
        lines.push(if allowed_hosts.is_empty() {
            "Hosts: any host (the manifest lists none)".to_string()
        } else {
            format!("Hosts: {}", allowed_hosts.join(", "))
        });
    }
    lines.push("Always decisions are saved for this plugin and workspace.".to_string());
    lines
}

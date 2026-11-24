//! Consent for a Python plugin's `net.http`, `fs.write` and `ai.query`
//! (stint 1489, decisions ledger row 103).
//!
//! Each use is an ordinary broker request: `PermissionMonitor` admits it from
//! its grant store at the moment of the effect, or files a pending ask that
//! waits for a person. The pane keeps only the list of asks it waits on; it
//! holds no answers. The in-pane prompt renders a broker pending id, and a
//! click settles exactly that id through `approve_pending`.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::app::permissions::Capability;
use crate::broker::gate::{Admission, AdmitRequest, PendingView, PermissionMonitor};
use crate::broker::principal::{AdmissionTrace, Principal};
use crate::broker::{ActorScope, TargetType};
use crate::host::guest_text;

/// The capabilities a Python plugin is asked about in its pane. Every other
/// sensitive capability keeps the launch-set rule until stint 1238 moves all
/// of them to broker admission.
pub(crate) const CONSENT_CAPABILITIES: [Capability; 3] = [
    Capability::NetHttp,
    Capability::FsWrite,
    Capability::AiQuery,
];

/// Asks one pane may wait on at once. A plugin that keeps asking (a poll
/// timer) cannot grow the list without bound.
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
    /// The host has not stamped this pane's owner, so no ask can name it.
    NoPane,
    /// A stored denial; `entry_id` is the row `plexi permissions allow` takes.
    Stored { entry_id: String },
    /// This request was denied ("Deny once", or its pane's ask was retired).
    Denied,
    /// The admission could not be recorded (revoked in between, audit failed).
    NotRecorded(String),
    /// Too many asks already wait in this pane.
    Backlog,
}

impl Refusal {
    fn mechanism(&self) -> &'static str {
        match self {
            Self::Undeclared => "python_undeclared",
            Self::NotHost => "python_not_host",
            Self::NoPane => "python_no_pane",
            Self::Stored { .. } => "grant_store",
            Self::Denied => "python_ask_denied",
            Self::NotRecorded(_) => "python_use_not_recorded",
            Self::Backlog => "python_consent_backlog",
        }
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
        Refusal::NoPane => {
            format!("{capability} is unavailable: the host has not attached this pane yet")
        }
        Refusal::Stored { entry_id } if !entry_id.is_empty() => format!(
            "{capability} is denied for {plugin_id}. To allow it, use the Permissions plugin \
             or run `plexi permissions allow {entry_id}`."
        ),
        Refusal::Stored { .. } => format!(
            "{capability} is denied for {plugin_id}. To allow it, use the Permissions plugin \
             or `plexi permissions list`."
        ),
        Refusal::Denied => {
            format!("{capability} was denied for this request. {plugin_id} asks again next time.")
        }
        Refusal::NotRecorded(error) => {
            format!("{capability} was refused: the approval could not be recorded ({error})")
        }
        Refusal::Backlog => format!(
            "{capability} request dropped: {MAX_PARKED} requests already wait for your answer"
        ),
    }
}

/// The approved host scope for `net.http`: the manifest's `allowed_hosts`
/// lowercased, trailing dot stripped, sorted, deduplicated and joined with
/// `,`; an empty list is `*`. The grant binds this string as its resource,
/// so any change to the list asks again. `http_host_allowed` enforces the
/// same slice per request, so the enforced list and the approved scope
/// cannot differ.
pub(crate) fn host_scope(allowed_hosts: &[String]) -> String {
    let mut hosts: Vec<String> = allowed_hosts
        .iter()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        .collect();
    hosts.sort();
    hosts.dedup();
    if hosts.is_empty() {
        "*".to_string()
    } else {
        hosts.join(",")
    }
}

/// Proof that the gate admitted this use. Only [`gate`] constructs it, and
/// every consent effect body takes one, so no handler can run a consent
/// effect without an admission.
pub(crate) struct Admitted {
    _private: (),
}

pub(crate) enum Gate {
    Admitted(Admitted),
    /// A pending ask waits for a person.
    Waiting(String),
    Refused(Refusal),
}

/// One guest request for a consent capability, as the host knows it.
pub(crate) struct Request<'a> {
    pub plugin_id: &'a str,
    /// Host-stamped `(pane_id, context_id)`; `None` until `set_pipe_owner`.
    pub owner: Option<(u64, u64)>,
    pub workspace_root: &'a Path,
    pub declared: &'a [String],
    pub allowed_hosts: &'a [String],
    pub capability: &'a str,
    /// The guest message; its `request_id` is not part of the binding.
    pub message: &'a Value,
    /// `pycap_<uuid>`, kept for the life of the request.
    pub call_id: &'a str,
    /// Asks this pane already waits on, not counting this request.
    pub waiting: usize,
}

impl Request<'_> {
    fn principal(&self) -> Principal {
        Principal::plugin_in_pane(self.plugin_id, self.owner.map_or(0, |(pane, _)| pane))
    }

    fn host_scope(&self) -> Option<String> {
        (self.capability == Capability::NetHttp.as_str()).then(|| host_scope(self.allowed_hosts))
    }

    /// `{"host_scope": .., "request": <fingerprint of the message without
    /// its request_id>}`. "Grant once" binds this fingerprint.
    fn input_json(&self) -> String {
        let mut message = self.message.clone();
        if let Some(object) = message.as_object_mut() {
            object.remove("request_id");
        }
        let digest = crate::broker::gate::fingerprint_args(&message.to_string())
            .unwrap_or_else(|error| format!("unfingerprinted:{error}"));
        let mut input = serde_json::json!({ "request": digest });
        if let Some(scope) = self.host_scope() {
            input["host_scope"] = Value::String(scope);
        }
        input.to_string()
    }

    fn trace(&self, decision: &str, mechanism: &str, outcome_ref: &str) {
        AdmissionTrace {
            principal: &self.principal(),
            effect: self.capability,
            resource_class: "capability",
            decision,
            mechanism,
            call_id: self.call_id,
            outcome_ref,
        }
        .emit();
    }
}

/// Decide one use of a consent capability. Refuses without asking when the
/// capability is undeclared, there is no monitor or pane owner, or the pane
/// already waits on [`MAX_PARKED`] asks. Otherwise the monitor decides; an
/// admitted use is recorded (`note_use`) and a one-shot grant spent before
/// the effect runs.
pub(crate) fn gate(monitor: Option<&PermissionMonitor>, request: &Request<'_>) -> Gate {
    let refuse = |refusal: Refusal| {
        request.trace("deny", refusal.mechanism(), "");
        Gate::Refused(refusal)
    };
    if !request
        .declared
        .iter()
        .any(|item| item == request.capability)
    {
        return refuse(Refusal::Undeclared);
    }
    let Some(monitor) = monitor else {
        return refuse(Refusal::NotHost);
    };
    let Some((pane_id, context_id)) = request.owner else {
        return refuse(Refusal::NoPane);
    };
    // Checked before the broker is asked, so a full pane files no ask that
    // nothing waits on.
    if request.waiting >= MAX_PARKED {
        return refuse(Refusal::Backlog);
    }
    let admission = admit_request(monitor, request, pane_id, context_id);
    match admission {
        Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            ..
        } => {
            if let Err(error) = monitor.note_use(
                &crate::broker::principal::RecordOwner::stored(
                    crate::broker::ActorType::Plugin,
                    request.plugin_id,
                ),
                request.call_id,
                &grant_id,
                &fingerprint,
                resource_id.as_deref().unwrap_or_default(),
                "",
            ) {
                return refuse(Refusal::NotRecorded(error));
            }
            // Spent at admission, so no second request can ride it.
            monitor.consume_once(&grant_id, None);
            Gate::Admitted(Admitted { _private: () })
        }
        Admission::Required { pending_request_id } => Gate::Waiting(pending_request_id),
        Admission::Denied { .. } => Gate::Refused(Refusal::Stored {
            entry_id: deny_entry_id(monitor, request),
        }),
    }
}

/// The broker admission for `request` from pane `pane_id` in `context_id`:
/// the exact request `gate` sends, without its pre-checks or use record.
pub(crate) fn admit_request(
    monitor: &PermissionMonitor,
    request: &Request<'_>,
    pane_id: u64,
    context_id: u64,
) -> Admission {
    let input = request.input_json();
    let disclosure = describe(
        request.plugin_id,
        request.capability,
        request.host_scope().as_deref(),
    )
    .join(" ");
    monitor.admit_plugin_capability(
        AdmitRequest {
            call_id: request.call_id,
            tool: request.capability,
            input_json: &input,
            principal: &Principal::plugin_in_pane(request.plugin_id, pane_id),
            actor_scope: ActorScope::User,
            trust_origin: "host",
            workspace_root: request.workspace_root,
            context_id,
            package_id: "",
            instance_id: pane_id,
            target_type: TargetType::Capability,
        },
        &disclosure,
    )
}

/// The stored denial that refused `request`, for `plexi permissions allow`.
fn deny_entry_id(monitor: &PermissionMonitor, request: &Request<'_>) -> String {
    let workspace = crate::platform::path::canonical_or_self(request.workspace_root)
        .display()
        .to_string();
    let scope = request.host_scope();
    let denials: Vec<_> = monitor
        .records_for(&request.principal())
        .entries()
        .into_iter()
        .filter(|entry| {
            entry.kind == "deny"
                && entry.actor_id == request.plugin_id
                && entry.tool == request.capability
                && entry.workspace == workspace
        })
        .collect();
    denials
        .iter()
        .find(|entry| entry.resource_id == scope)
        .or_else(|| denials.first())
        .map(|entry| entry.id.clone())
        .unwrap_or_default()
}

/// Trace and describe the refusal of a waiting request whose ask was denied.
pub(crate) fn refuse_denied(request: &Request<'_>) -> Refusal {
    request.trace("deny", Refusal::Denied.mechanism(), "");
    Refusal::Denied
}

/// One ask this pane waits on.
pub(crate) struct Waiting {
    pub pending_id: String,
    /// Fresh per ask shown; the consent guard arms on it.
    pub prompt_seq: u64,
    pub capability: String,
    pub message: Value,
    pub call_id: String,
}

/// One pane's consent state: the asks it waits on, and the last decision that
/// could not be saved. Lives exactly as long as the pane.
#[derive(Default)]
pub(crate) struct PaneConsent {
    /// The profile's monitor, already opened by the launch gate; `None`
    /// outside the host, where every consent capability is refused.
    pub(crate) monitor: Option<Arc<PermissionMonitor>>,
    pub(crate) waiting: Vec<Waiting>,
    /// `(pending_id, error)` of a click whose decision was not saved. The
    /// prompt stays open and shows it.
    pub(crate) last_error: Option<(String, String)>,
}

impl PaneConsent {
    pub(crate) fn for_profile() -> Self {
        Self {
            monitor: PermissionMonitor::loaded(&crate::config::config_dir()),
            ..Self::default()
        }
    }

    /// The first ask whose broker pending row still exists, with that row.
    pub(crate) fn prompt(&self, principal: &Principal) -> Option<(&Waiting, PendingView)> {
        let monitor = self.monitor.as_deref()?;
        self.waiting.iter().find_map(|waiting| {
            monitor
                .records_for(principal)
                .pending_id(&waiting.pending_id)
                .map(|view| (waiting, view))
        })
    }
}

/// The words of an ask, shared by the in-pane prompt and the broker's
/// disclosure. Every interpolated value is escaped.
fn describe(plugin_id: &str, capability: &str, host_scope: Option<&str>) -> Vec<String> {
    let plugin = guest_text::shown(plugin_id);
    let mut lines = vec![format!("{plugin} requests:")];
    match Capability::try_from(capability) {
        Ok(known) => lines.push(format!("{capability}: {}", known.description())),
        Err(_) => lines.push(guest_text::shown(capability)),
    }
    let days = crate::broker::gate::ALWAYS_TTL_SECS / (24 * 60 * 60);
    match host_scope {
        None => lines.push(format!(
            "Always allow saves this for {days} days, for {plugin} in this workspace."
        )),
        Some("*") => {
            lines.push("Hosts: any host (the manifest lists none)".to_string());
            lines.push(format!(
                "Always allow saves this for {days} days, for {plugin} in this workspace and any host."
            ));
        }
        Some(scope) => {
            let hosts: Vec<String> = scope.split(',').map(guest_text::shown).collect();
            // `http_host_allowed` also admits each host's subdomains.
            lines.push(format!("Hosts: {} and their subdomains", hosts.join(", ")));
            lines.push(format!(
                "Always allow saves this for {days} days, for {plugin} in this workspace and these hosts only."
            ));
        }
    }
    lines
}

/// The prompt body, one caption per line, read from the pending row: the
/// plugin, capability and host list the grant will be built from.
pub(crate) fn prompt_lines(view: &PendingView) -> Vec<String> {
    let scope = (view.tool == Capability::NetHttp.as_str())
        .then(|| view.resource_id.as_deref().unwrap_or_default());
    describe(&view.actor_id, &view.tool, scope)
}

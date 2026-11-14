//! Characterisation matrix of today's admission results (stint 1238, first
//! deliverable). Tests only: it pins what each runtime admits today so the
//! one-broker cutover diff shows exactly which cells change.
//!
//! Read [`TABLE`] as the matrix: one line per runtime x effect family, one
//! cell per grant state in [`STATES`] order. Each cell names the outcome and
//! the mechanism that decided it. A cell carrying `.cut(..)` (and a
//! `// CUTOVER:` comment on its line) is today's behavior that the design in
//! `docs/prm/permissions-broker.md` ("The one broker") says must change; the
//! string is the expected post-cutover result. Nothing here fixes anything.
//!
//! Drivers that observe the cells:
//! - Python plugin: `python_runtime_admission_matrix` in `src/host/wasm_python.rs`
//!   (needs the private `manifest_declares`).
//! - Native WASM plugin: `wasm_runtime_admission_matrix` in `src/host/wasm_pane.rs`
//!   (needs the private `WasmPane::exec` and its queue).
//! - CLI/agent adapter and the shared event-subscription path: this file.
//!
//! Standalone pins outside the drivers:
//! - `raw_set_permission_socket_line_writes_no_grant` in
//!   `src/testing/agent_security_tests.rs` (stint 1272): no raw socket line,
//!   from a pane or from no pane, writes a capability grant.
//! - `socket_caller_with_no_pane_is_not_human` in
//!   `src/testing/agent_security_tests.rs` (stint 1271): a socket caller whose
//!   ancestry reaches no pane is not human; its `allow`/`reset` file Needs you
//!   and only a desktop click applies them.
//!
//! Every driver uses `PermissionMonitor::ephemeral()` or an in-memory
//! `GrantStore` unless a cell is about persistence; those use a temp profile
//! with the test host-key backend. No network, no login keychain, no consent
//! committed through a synthetic UI route.

use std::path::Path;

use super::gate::{Admission, AdmitRequest, ApprovalChoice, PermissionMonitor};
use super::{
    ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord, GrantSource,
    GrantStore, PermissionPosture, TargetType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Runtime {
    /// CPython-in-WASM plugin pane (`LivePythonPane`).
    Python,
    /// Native WASM component plugin pane (`WasmPane`).
    Wasm,
    /// Agent pane or CLI caller reaching the broker as `ActorType::Agent`.
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    FsRead,
    FsWrite,
    Network,
    Secrets,
    CliExec,
    Ai,
    Clipboard,
    Events,
    Panes,
    /// Directed data pipes to other panes (`pipe.open`).
    Pipes,
    Notify,
    HostRead,
    PluginTools,
}

impl Family {
    pub(crate) const ALL: [Family; 13] = [
        Family::FsRead,
        Family::FsWrite,
        Family::Network,
        Family::Secrets,
        Family::CliExec,
        Family::Ai,
        Family::Clipboard,
        Family::Events,
        Family::Panes,
        Family::Pipes,
        Family::Notify,
        Family::HostRead,
        Family::PluginTools,
    ];
}

/// The brief's eight states, with `once` and `session` each split in two
/// because their replay and staleness behavior is exactly what the cutover
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrantState {
    /// Not declared, nothing stored.
    None,
    /// An unconsumed one-shot allow.
    OnceFresh,
    /// A one-shot allow that has already been consumed.
    OnceUsed,
    /// A session allow from the running host session.
    SessionCurrent,
    /// A session allow carrying an earlier host session's id.
    SessionStale,
    /// A remembered (`Always`) allow.
    Remembered,
    /// A remembered deny.
    Denied,
    /// A remembered allow, then revoked through the monitor.
    Revoked,
    /// A remembered allow whose `expires_at` has passed.
    Expired,
    /// Declared by the manifest (plugins) or allowed by the agent posture
    /// (agents), with nothing stored.
    ManifestOnly,
}

pub(crate) const STATES: [GrantState; 10] = [
    GrantState::None,
    GrantState::OnceFresh,
    GrantState::OnceUsed,
    GrantState::SessionCurrent,
    GrantState::SessionStale,
    GrantState::Remembered,
    GrantState::Denied,
    GrantState::Revoked,
    GrantState::Expired,
    GrantState::ManifestOnly,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Admit,
    /// A human decision is required before the effect runs.
    Ask,
    Deny,
}

/// Which mechanism decided a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mech {
    /// A stored `GrantStore` row, read once at launch by
    /// `PermissionMonitor::materialize_*` into the runtime's launch set.
    LaunchRow,
    /// The manifest declaration itself: `materialize_capability_ids`
    /// auto-minted an allow row (`GrantSource::Workspace`).
    ManifestAsGrant,
    /// `LivePythonPane::has_capability`: the id is absent from the launch
    /// snapshot and Python has no runtime consent path.
    PythonLaunchSet,
    /// `WasmPane`'s own `pending_capability_prompts` queue.
    WasmQueue,
    /// `PermissionMonitor::admit*` exact evaluation.
    MonitorAdmit,
    /// `HostSubscriptionService`'s private `GrantStore` copy through
    /// `event_subscriptions::evaluate_subscription`, outside `PermissionMonitor`.
    SubscriptionStore,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Cell {
    pub outcome: Outcome,
    pub mech: Mech,
    /// Expected post-cutover result when today's result contradicts the design.
    pub cutover: Option<&'static str>,
}

impl Cell {
    const fn cut(self, expected: &'static str) -> Self {
        Self {
            cutover: Some(expected),
            ..self
        }
    }
}

const fn admit(mech: Mech) -> Cell {
    Cell {
        outcome: Outcome::Admit,
        mech,
        cutover: None,
    }
}

const fn ask(mech: Mech) -> Cell {
    Cell {
        outcome: Outcome::Ask,
        mech,
        cutover: None,
    }
}

const fn deny(mech: Mech) -> Cell {
    Cell {
        outcome: Outcome::Deny,
        mech,
        cutover: None,
    }
}

pub(crate) enum Row {
    Cells([Cell; 10]),
    /// The runtime has no such effect. The reason names why.
    NotApplicable(&'static str),
}

pub(crate) struct Line {
    pub runtime: Runtime,
    pub family: Family,
    /// Ids the driver probes. Every id must produce the whole row. `<ws>`
    /// stands for the test workspace root.
    pub ids: &'static [&'static str],
    pub row: Row,
}

const ROW: Mech = Mech::LaunchRow;
const MINT: Mech = Mech::ManifestAsGrant;
const PYSET: Mech = Mech::PythonLaunchSet;
const QUEUE: Mech = Mech::WasmQueue;
const ADMIT: Mech = Mech::MonitorAdmit;
const SUBS: Mech = Mech::SubscriptionStore;

const ONCE_REPLAY: &str = "ask: a consumed one-shot must not authorize again";
const STALE_SESSION: &str = "ask: a grant from an earlier host session must not authorize";
const REMINT: &str = "ask: the manifest must not re-mint an allow after revoke or expiry";
const MANIFEST_MINT: &str = "ask: a manifest declaration must not mint an allow";
const PY_NO_ASK: &str = "ask (waiting_for_approval): Python has no consent path and hard-denies";
const RUNTIME_QUEUE: &str = "ask through a broker pending request, not the WasmPane queue";
const SUBS_STORE: &str =
    "same result decided by PermissionMonitor, not the subscription store copy";

// Column order for every row below (see STATES):
//   none, once, once used, session, stale session, remembered, denied, revoked, expired, manifest-only

// Declared non-sensitive manifest capability, Python.
// CUTOVER: once-used, stale session, revoked, expired and manifest-only admit.
#[rustfmt::skip]
const PY_NON_SENSITIVE: [Cell; 10] = [
    deny(PYSET), admit(ROW), admit(ROW).cut(ONCE_REPLAY), admit(ROW), admit(ROW).cut(STALE_SESSION),
    admit(ROW), deny(ROW), admit(MINT).cut(REMINT), admit(MINT).cut(REMINT), admit(MINT).cut(MANIFEST_MINT),
];

// Declared sensitive manifest capability, Python.
// CUTOVER: once-used and stale session admit; revoked, expired and manifest-only hard-deny.
#[rustfmt::skip]
const PY_SENSITIVE: [Cell; 10] = [
    deny(PYSET), admit(ROW), admit(ROW).cut(ONCE_REPLAY), admit(ROW), admit(ROW).cut(STALE_SESSION),
    admit(ROW), deny(ROW), deny(PYSET).cut(PY_NO_ASK), deny(PYSET).cut(PY_NO_ASK), deny(PYSET).cut(PY_NO_ASK),
];

// `notify` is a raw WASM id, not a `Capability`, so it can never enter the
// Python launch set: `ShowNotification` is denied in every state, even with a
// stored allow. Not a widening, so not marked; reported as a finding.
#[rustfmt::skip]
const PY_NOTIFY: [Cell; 10] = [
    deny(PYSET), deny(PYSET), deny(PYSET), deny(PYSET), deny(PYSET),
    deny(PYSET), deny(PYSET), deny(PYSET), deny(PYSET), deny(PYSET),
];

// Python typed CLI operation: `cli.operations` must be in the launch set
// (stage 1, auto-minted when declared because it only lets the plugin ask),
// then each invocation is admitted by the monitor against the exact operation
// binding (stage 2). Grant states apply to the stage-2 operation grant.
#[rustfmt::skip]
const PY_CLI_OPERATION: [Cell; 10] = [
    deny(PYSET), admit(ADMIT), ask(ADMIT), admit(ADMIT), ask(ADMIT),
    admit(ADMIT), deny(ADMIT), ask(ADMIT), ask(ADMIT), ask(ADMIT),
];

// Raw WASM capability that requires consent (every raw id the WIT surface
// gates through `request-capability`).
// CUTOVER: every ask comes from the WasmPane queue; once-used and stale session admit.
#[rustfmt::skip]
const WASM_CONSENT: [Cell; 10] = [
    ask(QUEUE).cut(RUNTIME_QUEUE), admit(ROW), admit(ROW).cut(ONCE_REPLAY), admit(ROW), admit(ROW).cut(STALE_SESSION),
    admit(ROW), deny(ROW), ask(QUEUE).cut(RUNTIME_QUEUE), ask(QUEUE).cut(RUNTIME_QUEUE), ask(QUEUE).cut(RUNTIME_QUEUE),
];

// Exact-binding admission through `PermissionMonitor::admit*`. Agent
// manifest-only means the agent posture lists the tool under `allow`, which is
// not authorization. Matches the design.
#[rustfmt::skip]
const EXACT: [Cell; 10] = [
    ask(ADMIT), admit(ADMIT), ask(ADMIT), admit(ADMIT), ask(ADMIT),
    admit(ADMIT), deny(ADMIT), ask(ADMIT), ask(ADMIT), ask(ADMIT),
];

// Event-stream subscription, shared by all three runtimes. Production calls
// `evaluate_subscription` with `session_id: None`, so a session-duration stream
// grant (the shape `allow_parent_child_lifecycle` writes) never matches.
// Manifest-only: no manifest field declares subscriptions; the posture `allow`
// list is the only declaration input and it does not authorize.
// CUTOVER: the deciding store is a copy outside PermissionMonitor (see
// `subscription_store_copy_misses_a_monitor_revocation`).
#[rustfmt::skip]
const EVENTS: [Cell; 10] = [
    ask(SUBS), admit(SUBS).cut(SUBS_STORE), ask(SUBS), ask(SUBS), ask(SUBS),
    admit(SUBS).cut(SUBS_STORE), deny(SUBS).cut(SUBS_STORE), ask(SUBS), ask(SUBS), ask(SUBS),
];

const fn line(
    runtime: Runtime,
    family: Family,
    ids: &'static [&'static str],
    cells: [Cell; 10],
) -> Line {
    Line {
        runtime,
        family,
        ids,
        row: Row::Cells(cells),
    }
}

const fn na(runtime: Runtime, family: Family, reason: &'static str) -> Line {
    Line {
        runtime,
        family,
        ids: &[],
        row: Row::NotApplicable(reason),
    }
}

use Family as F;
use Runtime as R;

/// The matrix. 3 runtimes x 13 families x 10 grant states.
pub(crate) const TABLE: &[Line] = &[
    // ── Python plugin (LivePythonPane) ───────────────────────────────────────
    line(R::Python, F::FsRead, &["fs.read", "fs.pick"], PY_NON_SENSITIVE),
    line(R::Python, F::FsWrite, &["fs.write"], PY_SENSITIVE),
    line(R::Python, F::Network, &["net.http"], PY_SENSITIVE),
    na(R::Python, F::Secrets, "secrets.get is declarable but LivePythonPane has no secret effect; nothing reaches admission"),
    line(R::Python, F::CliExec, &["cli.operations"], PY_CLI_OPERATION),
    line(R::Python, F::Ai, &["ai.query"], PY_SENSITIVE),
    na(R::Python, F::Clipboard, "the Python bridge has no clipboard effect"),
    line(R::Python, F::Events, &["publisher.plugin::changed"], EVENTS),
    line(
        R::Python,
        F::Panes,
        &["panes.read", "panes.control", "panes.spawn", "spawn.plugin", "terminal.bindings", "permissions.manage"],
        PY_SENSITIVE,
    ),
    line(R::Python, F::Pipes, &["pipe.open"], PY_NON_SENSITIVE),
    line(R::Python, F::Notify, &["notify"], PY_NOTIFY),
    line(R::Python, F::HostRead, &["logs.read", "mcp.client"], PY_NON_SENSITIVE),
    na(R::Python, F::PluginTools, "plugins publish connector tools; no plugin-to-plugin tool call reaches the broker"),
    // ── Native WASM plugin (WasmPane) ────────────────────────────────────────
    line(R::Wasm, F::FsRead, &["fs:read:<ws>"], WASM_CONSENT),
    line(R::Wasm, F::FsWrite, &["fs:write:<ws>"], WASM_CONSENT),
    line(R::Wasm, F::Network, &["net:fetch:api.test"], WASM_CONSENT),
    na(R::Wasm, F::Secrets, "validate_wasm_capability_id has no secret id; the WIT surface has no secret effect"),
    na(R::Wasm, F::CliExec, "no WIT cli-operation effect; spawn-child is recorded without runtime access (apply_capability_grant)"),
    line(R::Wasm, F::Ai, &["ai.query"], WASM_CONSENT),
    line(R::Wasm, F::Clipboard, &["clipboard.read", "clipboard.write"], WASM_CONSENT),
    line(R::Wasm, F::Events, &["publisher.plugin::changed"], EVENTS),
    line(R::Wasm, F::Panes, &["open-pane", "spawn.plugin"], WASM_CONSENT),
    na(R::Wasm, F::Pipes, "pipe.open is granted at load through Grants (manifest pipe.open), not request-capability"),
    line(R::Wasm, F::Notify, &["notify"], WASM_CONSENT),
    na(R::Wasm, F::HostRead, "the WIT surface has no host log or MCP effect"),
    na(R::Wasm, F::PluginTools, "plugins publish connector tools; no plugin-to-plugin tool call reaches the broker"),
    // ── CLI/agent adapter (ActorType::Agent) ─────────────────────────────────
    na(R::Agent, F::FsRead, "the harness reads files natively; no brokered read tool exists"),
    line(R::Agent, F::FsWrite, &["host.files.edit"], EXACT),
    na(R::Agent, F::Network, "the harness makes network calls natively; no brokered network tool exists"),
    na(R::Agent, F::Secrets, "no production admit call site for secret.get or secret.read (resource_of only reserves the names)"),
    na(R::Agent, F::CliExec, "the harness executes in its own PTY; typed CLI operations are plugin-only (TargetType::CliOperation)"),
    na(R::Agent, F::Ai, "agents use their own provider; no brokered ai tool for agents"),
    na(R::Agent, F::Clipboard, "no brokered clipboard tool for agents"),
    line(R::Agent, F::Events, &["publisher.plugin::changed"], EVENTS),
    line(R::Agent, F::Panes, &["pane.send"], EXACT),
    na(R::Agent, F::Pipes, "agents have no directed-pipe effect; pipes are plugin-to-plugin"),
    na(R::Agent, F::Notify, "plexi notify has no admission call site"),
    na(R::Agent, F::HostRead, "plexi log reads have no admission call site"),
    line(R::Agent, F::PluginTools, &["probe.add"], EXACT),
];

/// Persistence probe `InstallYes` (stint 1291): what a plugin gets at first
/// launch after `plexi plugin install --yes`. On alpha before stint 1291 the
/// CLI sealed an Allow row for every id in `[plugin.capabilities.wasm]`
/// `required`, for any runtime: a WASM package listing these ids, or a Python
/// package listing `spawn.plugin` in that table, admitted with no click. (A
/// Python package declaring `spawn.plugin` only under `[plugin.capabilities]`
/// got no row and was withheld, then as now.) Now install writes no grant and
/// a Python package carrying the table is refused at validation, so each line
/// is declared-and-nothing-stored. Driven by `install_yes_probe`.
pub(crate) const INSTALL_YES: &[(Runtime, Family, &[&str], Cell)] = &[
    (
        R::Python,
        F::Panes,
        &["spawn.plugin"],
        deny(PYSET).cut(PY_NO_ASK),
    ),
    (
        R::Wasm,
        F::Panes,
        &["open-pane", "spawn.plugin"],
        ask(QUEUE).cut(RUNTIME_QUEUE),
    ),
];

/// Ids each plugin runtime's registry knows that no line probes, with the
/// reason. The coverage test fails if a registry id is in neither place.
pub(crate) const PYTHON_UNPROBED: &[(&str, &str)] = &[
    (
        "secrets.get",
        "no Python effect (Secrets line is not-applicable)",
    ),
    (
        "gpu.render",
        "not consulted through LivePythonPane::has_capability",
    ),
    (
        "timer",
        "not consulted through LivePythonPane::has_capability",
    ),
    (
        "audio.record",
        "media_io release gate withholds it before admission",
    ),
    (
        "audio.playback",
        "media_io release gate withholds it before admission",
    ),
    (
        "video.playback",
        "media_io release gate withholds it before admission",
    ),
    (
        "midi.in",
        "media_io release gate withholds it before admission",
    ),
    (
        "midi.out",
        "media_io release gate withholds it before admission",
    ),
];

pub(crate) const WASM_UNPROBED: &[(&str, &str)] = &[
    (
        "state:read-write",
        "host store access granted at load through Grants, not request-capability",
    ),
    (
        "pipe.open",
        "granted at load through Grants (manifest pipe.open), not request-capability",
    ),
    (
        "gpu.render",
        "granted at load through Grants (manifest gpu.render), not request-capability",
    ),
    (
        "audio.playback",
        "granted at load through Grants, media_io release gated",
    ),
    (
        "audio:record",
        "media_io release gated; no capture effect reaches admission without it",
    ),
    (
        "spawn-child",
        "recorded without runtime access (CliExec line is not-applicable)",
    ),
];

/// (runtime, family) pairs each driver observes. The coverage test checks
/// this against the table, so a line no driver checks fails loudly.
pub(crate) const DRIVEN: &[(Runtime, Family, &str)] = &[
    (R::Python, F::FsRead, "python_runtime_admission_matrix"),
    (R::Python, F::FsWrite, "python_runtime_admission_matrix"),
    (R::Python, F::Network, "python_runtime_admission_matrix"),
    (R::Python, F::Ai, "python_runtime_admission_matrix"),
    (R::Python, F::CliExec, "python_runtime_admission_matrix"),
    (R::Python, F::Panes, "python_runtime_admission_matrix"),
    (R::Python, F::Pipes, "python_runtime_admission_matrix"),
    (R::Python, F::Notify, "python_runtime_admission_matrix"),
    (R::Python, F::HostRead, "python_runtime_admission_matrix"),
    (R::Wasm, F::FsRead, "wasm_runtime_admission_matrix"),
    (R::Wasm, F::FsWrite, "wasm_runtime_admission_matrix"),
    (R::Wasm, F::Network, "wasm_runtime_admission_matrix"),
    (R::Wasm, F::Ai, "wasm_runtime_admission_matrix"),
    (R::Wasm, F::Clipboard, "wasm_runtime_admission_matrix"),
    (R::Wasm, F::Panes, "wasm_runtime_admission_matrix"),
    (R::Wasm, F::Notify, "wasm_runtime_admission_matrix"),
    (R::Agent, F::FsWrite, "agent_adapter_admission_matrix"),
    (R::Agent, F::Panes, "agent_adapter_admission_matrix"),
    (R::Agent, F::PluginTools, "agent_adapter_admission_matrix"),
    (R::Python, F::Events, "event_subscription_admission_matrix"),
    (R::Wasm, F::Events, "event_subscription_admission_matrix"),
    (R::Agent, F::Events, "event_subscription_admission_matrix"),
];

/// Compare every cell of the given runtime and families with `observe` and
/// fail once, listing every mismatch. Returns the number of cells checked.
pub(crate) fn verify(
    runtime: Runtime,
    families: &[Family],
    ws: &Path,
    mut observe: impl FnMut(Family, &str, GrantState) -> (Outcome, Mech),
) -> usize {
    let mut checked = 0;
    let mut mismatches = Vec::new();
    for family in families {
        let line = TABLE
            .iter()
            .find(|line| line.runtime == runtime && line.family == *family)
            .unwrap_or_else(|| panic!("no matrix line for {runtime:?} {family:?}"));
        let Row::Cells(cells) = &line.row else {
            panic!("{runtime:?} {family:?} is not-applicable; no driver may observe it");
        };
        for raw_id in line.ids {
            let id = raw_id.replace("<ws>", &ws.display().to_string());
            for (state, cell) in STATES.iter().zip(cells.iter()) {
                let observed = observe(*family, &id, *state);
                checked += 1;
                if observed != (cell.outcome, cell.mech) {
                    mismatches.push(format!(
                        "{runtime:?} {family:?} {id} {state:?}: table says {:?} by {:?}, observed {:?} by {:?}",
                        cell.outcome, cell.mech, observed.0, observed.1
                    ));
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} admission cell(s) differ from the characterisation table:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    checked
}

// ── Shared seeding ───────────────────────────────────────────────────────────

pub(crate) const PREVIOUS_SESSION: &str = "previous-host-session";

/// True when a plugin in `state` declares the capability in its manifest.
pub(crate) fn plugin_declares(state: GrantState) -> bool {
    state != GrantState::None
}

/// Store the capability row a plugin in `state` has before launch, exactly as
/// the monitor's own writers shape it (`GrantRecord::capability_id`).
pub(crate) fn seed_plugin_capability(
    monitor: &PermissionMonitor,
    plugin_id: &str,
    ws: &Path,
    capability_id: &str,
    state: GrantState,
) {
    let row = |decision| {
        GrantRecord::capability_id(plugin_id, ws, capability_id, decision, GrantSource::User)
    };
    let mut record = match state {
        GrantState::None | GrantState::ManifestOnly => return,
        GrantState::Denied => row(Decision::Deny),
        _ => row(Decision::Allow),
    };
    match state {
        GrantState::OnceFresh => record.duration = GrantDuration::Once,
        GrantState::OnceUsed => {
            record.duration = GrantDuration::Once;
            record.consumed = true;
        }
        GrantState::SessionCurrent => {
            record.duration = GrantDuration::Session;
            record.session_id = Some(monitor.session_id().to_string());
        }
        GrantState::SessionStale => {
            record.duration = GrantDuration::Session;
            record.session_id = Some(PREVIOUS_SESSION.to_string());
        }
        GrantState::Expired => record.expires_at = Some(monitor.now_secs() - 60),
        _ => {}
    }
    let grant_id = record.grant_id.clone();
    monitor.store().record(record);
    if state == GrantState::Revoked {
        assert!(monitor.revoke_grant_id(&grant_id), "revoke {grant_id}");
    }
}

/// True when `materialize_*` minted an allow row from the declaration alone.
pub(crate) fn manifest_minted(monitor: &PermissionMonitor, plugin_id: &str, id: &str) -> bool {
    monitor.store().records().iter().any(|record| {
        record.actor_id == plugin_id
            && record.target_id == id
            && record.decision == Decision::Allow
            && record.source == GrantSource::Workspace
    })
}

pub(crate) fn outcome_of(admission: &Admission) -> Outcome {
    match admission {
        Admission::Proceed { .. } => Outcome::Admit,
        Admission::Required { .. } => Outcome::Ask,
        Admission::Denied { .. } => Outcome::Deny,
    }
}

fn pending_id(admission: Admission, what: &str) -> String {
    match admission {
        Admission::Required { pending_request_id } => pending_request_id,
        Admission::Proceed { .. } => panic!("{what}: expected an ask, got Proceed"),
        Admission::Denied { code } => panic!("{what}: expected an ask, got Denied({code})"),
    }
}

/// Drive an exact-binding admission into `state` through the monitor's own
/// decision functions (`approve_pending`, `consume_once`, `revoke_grant_id`),
/// then return the outcome of the next identical request. `admit` receives the
/// posture to apply; only `ManifestOnly` supplies one.
pub(crate) fn drive_exact(
    monitor: &PermissionMonitor,
    state: GrantState,
    tool: &str,
    admit: &dyn Fn(Option<&PermissionPosture>) -> Admission,
) -> Outcome {
    let approve = |choice| {
        let id = pending_id(admit(None), tool);
        monitor
            .approve_pending(&id, choice)
            .unwrap_or_else(|error| panic!("{tool}: approve {choice:?}: {error}"));
    };
    let edit_allow = |duration: GrantDuration, edit: &dyn Fn(&mut GrantRecord)| {
        let mut store = monitor.store();
        let record = store
            .records_mut()
            .iter_mut()
            .find(|record| {
                record.target_id == tool
                    && record.decision == Decision::Allow
                    && record.duration == duration
            })
            .unwrap_or_else(|| panic!("{tool}: no {duration:?} allow to edit"));
        edit(record);
    };
    match state {
        GrantState::None => {}
        GrantState::OnceFresh => approve(ApprovalChoice::Once),
        GrantState::OnceUsed => {
            approve(ApprovalChoice::Once);
            let Admission::Proceed { grant_id, .. } = admit(None) else {
                panic!("{tool}: a fresh once grant must proceed before it is consumed");
            };
            monitor.consume_once(&grant_id, None);
        }
        GrantState::SessionCurrent => approve(ApprovalChoice::Session),
        GrantState::SessionStale => {
            approve(ApprovalChoice::Session);
            edit_allow(GrantDuration::Session, &|record| {
                record.session_id = Some(PREVIOUS_SESSION.to_string());
            });
        }
        GrantState::Remembered => approve(ApprovalChoice::Always),
        GrantState::Denied => approve(ApprovalChoice::DenyAlways),
        GrantState::Revoked => {
            approve(ApprovalChoice::Always);
            let Admission::Proceed { grant_id, .. } = admit(None) else {
                panic!("{tool}: a remembered grant must proceed before it is revoked");
            };
            assert!(monitor.revoke_grant_id(&grant_id), "{tool}: revoke");
        }
        GrantState::Expired => {
            approve(ApprovalChoice::Always);
            let past = monitor.now_secs() - 60;
            edit_allow(GrantDuration::Always, &|record| {
                record.expires_at = Some(past);
            });
        }
        GrantState::ManifestOnly => {
            let posture = PermissionPosture {
                default_posture: Decision::Ask,
                allow: vec![tool.to_string()],
                ask: Vec::new(),
                deny: Vec::new(),
            };
            return outcome_of(&admit(Some(&posture)));
        }
    }
    outcome_of(&admit(None))
}

// ── Coverage ─────────────────────────────────────────────────────────────────

#[test]
fn matrix_has_one_line_per_runtime_and_family_and_every_line_is_driven() {
    for runtime in [Runtime::Python, Runtime::Wasm, Runtime::Agent] {
        for family in Family::ALL {
            let lines: Vec<&Line> = TABLE
                .iter()
                .filter(|line| line.runtime == runtime && line.family == family)
                .collect();
            assert_eq!(
                lines.len(),
                1,
                "{runtime:?} {family:?} needs exactly one line"
            );
            let driven = DRIVEN.iter().any(|(r, f, _)| *r == runtime && *f == family);
            match &lines[0].row {
                Row::Cells(_) => {
                    assert!(driven, "{runtime:?} {family:?} has cells but no driver");
                    assert!(
                        !lines[0].ids.is_empty(),
                        "{runtime:?} {family:?} probes no id"
                    );
                }
                Row::NotApplicable(reason) => {
                    assert!(
                        !driven,
                        "{runtime:?} {family:?} is not-applicable but driven"
                    );
                    assert!(!reason.is_empty(), "{runtime:?} {family:?} needs a reason");
                }
            }
        }
    }
    assert_eq!(TABLE.len(), 39);
}

#[test]
fn every_registry_capability_is_probed_or_explained() {
    let probed = |runtime: Runtime, id: &str| {
        TABLE
            .iter()
            .filter(|line| line.runtime == runtime)
            .any(|line| line.ids.contains(&id))
    };
    for cap in crate::app::permissions::Capability::ALL {
        let id = cap.as_str();
        let explained = PYTHON_UNPROBED.iter().any(|(unprobed, _)| *unprobed == id);
        assert!(
            probed(Runtime::Python, id) != explained,
            "Python capability {id} must be either probed or explained, not both or neither"
        );
    }
    let raw_forms = [
        "ai.query",
        "state:read-write",
        "pipe.open",
        "gpu.render",
        "audio.playback",
        "fs:read:<ws>",
        "fs:write:<ws>",
        "net:fetch:api.test",
        "audio:record",
        "spawn-child",
        "spawn.plugin",
        "clipboard.read",
        "clipboard.write",
        "notify",
        "open-pane",
    ];
    for id in raw_forms {
        let concrete = id.replace("<ws>", "/ws");
        assert!(
            crate::app::permissions::validate_wasm_capability_id(&concrete).is_ok(),
            "{id} is not a current raw WASM id"
        );
        let explained = WASM_UNPROBED.iter().any(|(unprobed, _)| *unprobed == id);
        assert!(
            probed(Runtime::Wasm, id) != explained,
            "raw WASM id {id} must be either probed or explained, not both or neither"
        );
    }
}

// ── CLI/agent adapter driver ─────────────────────────────────────────────────

/// The three agent call sites that reach `PermissionMonitor::admit*` today,
/// with the request shape each production caller builds: `host.files.edit`
/// (`host::changes::admit_edit`), `pane.send` (`App` pane-send handling in
/// `app/lifecycle.rs`) and a plugin connector tool (`app::plugin_call`).
#[test]
fn agent_adapter_admission_matrix() {
    let profile = tempfile::tempdir().unwrap();
    let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
    let ws = tempfile::tempdir().unwrap();
    let ws = ws.path();
    let checked = verify(
        Runtime::Agent,
        &[Family::FsWrite, Family::Panes, Family::PluginTools],
        ws,
        |family, tool, state| {
            let monitor = PermissionMonitor::ephemeral();
            let (target_type, actor_scope, actor_id, package_id, input) = match family {
                Family::FsWrite => (
                    TargetType::HostTool,
                    ActorScope::User,
                    "agent:editor",
                    "",
                    r#"{"path":"notes.md","edit_fingerprint":"e1"}"#,
                ),
                Family::Panes => (
                    TargetType::Capability,
                    ActorScope::Workspace,
                    "pane:4",
                    "",
                    r#"{"pane_id":7,"submit":true,"paste":false,"text_fingerprint":"t1","operation_id":"op-1"}"#,
                ),
                Family::PluginTools => (
                    TargetType::PluginConnector,
                    ActorScope::User,
                    "agent:caller",
                    "probe",
                    "{}",
                ),
                other => panic!("agent driver does not own {other:?}"),
            };
            let outcome = drive_exact(&monitor, state, tool, &|posture| {
                monitor.admit_with_posture(
                    AdmitRequest {
                        call_id: "call-1",
                        tool,
                        input_json: input,
                        principal: &super::principal::Principal::agent(actor_id),
                        actor_scope,
                        trust_origin: "host",
                        workspace_root: ws,
                        context_id: 1,
                        package_id,
                        instance_id: 0,
                        target_type,
                    },
                    posture,
                )
            });
            (outcome, Mech::MonitorAdmit)
        },
    );
    assert_eq!(checked, 30);
}

// ── Event subscription driver (all runtimes) ─────────────────────────────────

const PUBLISHER: &str = "publisher.plugin";
const EVENT: &str = "changed";

fn subscriber(runtime: Runtime) -> (ActorType, &'static str) {
    match runtime {
        Runtime::Python => (ActorType::Plugin, "python.subscriber"),
        Runtime::Wasm => (ActorType::Plugin, "wasm.subscriber"),
        Runtime::Agent => (ActorType::Agent, "pane:4"),
    }
}

fn stream_grant(
    runtime: Runtime,
    ws: &Path,
    target: &str,
    duration: GrantDuration,
    session_id: Option<String>,
) -> GrantRecord {
    let (actor_type, actor_id) = subscriber(runtime);
    GrantRecord::event_stream_allow(
        actor_type,
        actor_id,
        ActorScope::User,
        target,
        ws,
        duration,
        GrantSource::User,
        session_id,
    )
}

/// Evaluate a subscription the way `HostSubscriptionService::classify_subscribe_request`
/// does: its own store, `ActorScope::User`, and no session id.
fn evaluate_stream(
    store: &GrantStore,
    posture: Option<&PermissionPosture>,
    runtime: Runtime,
    ws: &Path,
) -> Decision {
    let (actor_type, actor_id) = subscriber(runtime);
    crate::host::event_subscriptions::evaluate_subscription(
        store,
        posture,
        Some(ws),
        PUBLISHER,
        &super::principal::Principal::from_actor(actor_type, actor_id, None)
            .expect("matrix subscriber is a requesting principal"),
        ActorScope::User,
        None,
        &[EVENT.to_string()],
    )
}

fn observe_stream(runtime: Runtime, ws: &Path, target: &str, state: GrantState) -> Outcome {
    let mut store = GrantStore::default();
    let now = crate::platform::clock::now_secs() as i64;
    let mut posture = None;
    match state {
        GrantState::None => {}
        GrantState::OnceFresh => {
            store.record(stream_grant(runtime, ws, target, GrantDuration::Once, None));
        }
        GrantState::OnceUsed => {
            let mut record = stream_grant(runtime, ws, target, GrantDuration::Once, None);
            record.consumed = true;
            store.record(record);
        }
        // The shape `allow_parent_child_lifecycle` writes: session, no session id.
        GrantState::SessionCurrent => {
            store.record(stream_grant(
                runtime,
                ws,
                target,
                GrantDuration::Session,
                None,
            ));
        }
        GrantState::SessionStale => store.record(stream_grant(
            runtime,
            ws,
            target,
            GrantDuration::Session,
            Some(PREVIOUS_SESSION.to_string()),
        )),
        GrantState::Remembered => {
            store.record(stream_grant(
                runtime,
                ws,
                target,
                GrantDuration::Always,
                None,
            ));
        }
        GrantState::Denied => {
            let (actor_type, actor_id) = subscriber(runtime);
            let binding = ExactBinding::event_stream(
                actor_type,
                actor_id,
                ActorScope::User,
                target,
                ws,
                None,
            );
            store.record(GrantRecord::from_binding(
                &binding,
                Decision::Deny,
                GrantDuration::Always,
                GrantSource::User,
                "deny-stream",
            ));
        }
        GrantState::Revoked => {
            let record = stream_grant(runtime, ws, target, GrantDuration::Always, None);
            let grant_id = record.grant_id.clone();
            store.record(record);
            store
                .records_mut()
                .retain(|record| record.grant_id != grant_id);
        }
        GrantState::Expired => {
            let mut record = stream_grant(runtime, ws, target, GrantDuration::Always, None);
            record.expires_at = Some(now - 60);
            store.record(record);
        }
        GrantState::ManifestOnly => {
            posture = Some(PermissionPosture {
                default_posture: Decision::Ask,
                allow: vec![target.to_string()],
                ask: Vec::new(),
                deny: Vec::new(),
            });
        }
    }
    match evaluate_stream(&store, posture.as_ref(), runtime, ws) {
        Decision::Allow => Outcome::Admit,
        Decision::Ask => Outcome::Ask,
        Decision::Deny => Outcome::Deny,
    }
}

#[test]
fn event_subscription_admission_matrix() {
    let ws = tempfile::tempdir().unwrap();
    let ws = ws.path();
    for runtime in [Runtime::Python, Runtime::Wasm, Runtime::Agent] {
        let checked = verify(runtime, &[Family::Events], ws, |_, target, state| {
            (
                observe_stream(runtime, ws, target, state),
                Mech::SubscriptionStore,
            )
        });
        assert_eq!(checked, 10, "{runtime:?}");
    }
}

/// CUTOVER: expected to change to ask. `HostSubscriptionService` decides from
/// its own `GrantStore` loaded from disk, so a revocation made through
/// `PermissionMonitor` is not seen until that copy is reloaded.
#[test]
fn subscription_store_copy_misses_a_monitor_revocation() {
    let profile = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let target = format!("{PUBLISHER}::{EVENT}");
    let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
    let record = stream_grant(
        Runtime::Agent,
        ws.path(),
        &target,
        GrantDuration::Always,
        None,
    );
    let grant_id = record.grant_id.clone();
    monitor.store().record(record);
    monitor.store().try_save().expect("save grants");

    let service_copy = GrantStore::load_or_default(profile.path()).unwrap();
    assert_eq!(
        evaluate_stream(&service_copy, None, Runtime::Agent, ws.path()),
        Decision::Allow
    );
    assert!(monitor.revoke_grant_id(&grant_id));
    // CUTOVER: expected to change to ask.
    assert_eq!(
        evaluate_stream(&service_copy, None, Runtime::Agent, ws.path()),
        Decision::Allow,
        "today the copy still admits after the monitor revoked the grant"
    );
    let reloaded = GrantStore::load_or_default(profile.path()).unwrap();
    assert_eq!(
        evaluate_stream(&reloaded, None, Runtime::Agent, ws.path()),
        Decision::Ask
    );
}

/// Every cell the design says must change, one markdown row each. Run with
/// `--nocapture` to print the PR-body table. The count is pinned so a cutover
/// that changes a marked cell must also update this test.
#[test]
fn cutover_cells_are_enumerated() {
    let mut rows = Vec::new();
    for line in TABLE {
        let Row::Cells(cells) = &line.row else {
            continue;
        };
        for (state, cell) in STATES.iter().zip(cells.iter()) {
            if let Some(expected) = cell.cutover {
                rows.push(format!(
                    "| {:?} | {:?} | {} | {state:?} | {:?} by {:?} | {expected} |",
                    line.runtime,
                    line.family,
                    line.ids.join(", "),
                    cell.outcome,
                    cell.mech
                ));
            }
        }
    }
    for (runtime, family, ids, cell) in INSTALL_YES {
        if let Some(expected) = cell.cutover {
            rows.push(format!(
                "| {runtime:?} | {family:?} | {} | InstallYes | {:?} by {:?} | {expected} |",
                ids.join(", "),
                cell.outcome,
                cell.mech
            ));
        }
    }
    println!("| Runtime | Family | Ids | State | Today | Expected after cutover |");
    println!("|---|---|---|---|---|---|");
    for row in &rows {
        println!("{row}");
    }
    // 86 matrix cells (the Python Ai line, stint 1450, adds the five
    // `PY_SENSITIVE` cutovers) plus the two `InstallYes` probes (stint 1291).
    assert_eq!(rows.len(), 88);
}

/// Stint 1291 `InstallYes` probe: install each package with `--yes` through
/// the real CLI install path, then read what its first launch would get.
#[test]
fn install_yes_probe() {
    let profile = tempfile::tempdir().unwrap();
    let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
    for (runtime, family, ids, cell) in INSTALL_YES {
        let src = tempfile::tempdir().unwrap();
        let id = format!("install-yes-{runtime:?}-{family:?}").to_lowercase();
        let quoted = ids
            .iter()
            .map(|id| format!("\"{id}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let (caps, runtime_table, entry) = match runtime {
            Runtime::Python => (
                format!("capabilities = [{quoted}]\n"),
                "kind = \"python\"\nentry = \"main.py\"",
                "main.py",
            ),
            Runtime::Wasm => (
                format!("capabilities = []\n\n[plugin.capabilities.wasm]\nrequired = [{quoted}]\n"),
                "kind = \"wasm\"\nentry = \"app.wasm\"",
                "app.wasm",
            ),
            Runtime::Agent => panic!("agents are not installed"),
        };
        std::fs::write(
            src.path().join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"{id}\"\nname = \"Probe\"\nversion = \"0.1.0\"\ndescription = \"Probe\"\n\n[plugin.capabilities]\n{caps}\n[runtime]\n{runtime_table}\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        std::fs::write(src.path().join(entry), b"\0asm").unwrap();
        let cwd = std::env::current_dir().unwrap();
        if *runtime == Runtime::Python {
            // The alpha route: the same ids in the WASM table of a Python
            // package. Refused now, and nothing is stored.
            let table = tempfile::tempdir().unwrap();
            std::fs::write(
                table.path().join("manifest.toml"),
                format!(
                    "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"{id}-table\"\nname = \"Probe\"\nversion = \"0.1.0\"\ndescription = \"Probe\"\n\n[plugin.capabilities]\ncapabilities = []\n\n[plugin.capabilities.wasm]\nrequired = [{quoted}]\n\n[runtime]\n{runtime_table}\n\n[[contributes.views]]\nid = \"{id}-table.main\"\ndefault = true\n"
                ),
            )
            .unwrap();
            std::fs::write(table.path().join(entry), b"\0asm").unwrap();
            let refused = crate::cli::plugin::plugin_install_with_pin(
                &table.path().to_string_lossy(),
                None,
                crate::cli::plugin::InstallConfirm::Interactive,
                true,
            );
            assert_eq!(
                refused, 1,
                "a Python package with a WASM table must be refused"
            );
            let grants =
                std::fs::read_to_string(profile.path().join("grants.toml")).unwrap_or_default();
            assert!(!grants.contains(&format!("{id}-table")), "{grants}");
        }
        let code = crate::cli::plugin::plugin_install_with_pin(
            &src.path().to_string_lossy(),
            None,
            crate::cli::plugin::InstallConfirm::Interactive,
            true,
        );
        assert_eq!(code, 0, "{runtime:?} probe package must install");
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        for probe in *ids {
            let outcome = match runtime {
                Runtime::Python => {
                    let cap = crate::app::permissions::Capability::try_from(*probe).unwrap();
                    let declared = std::iter::once(cap).collect();
                    let granted = monitor
                        .materialize_plugin_permissions(
                            &super::principal::PluginInstance::new(id.as_str(), None),
                            &cwd,
                            &declared,
                            Vec::new(),
                        )
                        .capabilities
                        .contains(&cap);
                    if granted {
                        Outcome::Admit
                    } else {
                        Outcome::Deny
                    }
                }
                _ => {
                    let declared = std::iter::once(probe.to_string()).collect();
                    let (granted, blocked) = monitor.materialize_wasm_sets(
                        &super::principal::PluginInstance::new(id.as_str(), None),
                        &cwd,
                        &declared,
                    );
                    if blocked.contains(*probe) {
                        Outcome::Deny
                    } else if granted.contains(*probe) {
                        Outcome::Admit
                    } else {
                        Outcome::Ask
                    }
                }
            };
            assert_eq!(
                outcome, cell.outcome,
                "{runtime:?} {family:?} {probe} after install --yes"
            );
        }
    }
}

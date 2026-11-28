//! The one admission entry for the command socket (stint 1285). Every socket
//! operation, a `HostRequest` or a streaming frame outside it, has exactly
//! one policy here, and every refusal is traced with the operation, the
//! caller class and the outcome. The match has no wildcard: a new request
//! does not compile until it is classified.

use crate::broker::caller::SocketCaller;
use plexi_protocol::HostRequest;

/// What a socket caller needs before its request reaches a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerbPolicy {
    /// Liveness (build info, wake), or a handler that answers every caller
    /// class itself: any caller, even unidentified.
    Open,
    /// Pane driving and observation. Any identified caller; attributed to
    /// it. Not a boundary between same-user processes until v1.1.
    Operator,
    /// Reaches secrets, grants, tools or another principal's state. The
    /// handler admits it through the broker as the caller's own principal;
    /// an outside `pane send` is attributed to the external caller and
    /// dispatched, like `pane new` (ruling 192).
    Protected,
    /// Widens authority. The handler files a Needs-you request (or finds
    /// the pending one) and returns its id; only a desktop click applies it.
    Widening,
    /// A human decision. Never applied from the socket.
    Decision,
    /// Reports for the caller's own pane: the caller must be a pane, and
    /// the handler checks the target is that pane.
    OwnPane,
    /// A plugin-plane request with no socket handler.
    NotOnSocket,
}

impl VerbPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Operator => "operator",
            Self::Protected => "protected",
            Self::Widening => "widening",
            Self::Decision => "decision",
            Self::OwnPane => "own_pane",
            Self::NotOnSocket => "not_on_socket",
        }
    }
}

/// `plexi permissions <op>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionsOp {
    List,
    Revoke,
    Allow,
    Reset,
}

impl PermissionsOp {
    pub(crate) fn parse(op: &str) -> Option<Self> {
        Some(match op {
            "list" => Self::List,
            "revoke" => Self::Revoke,
            "allow" => Self::Allow,
            "reset" => Self::Reset,
            _ => return None,
        })
    }
}

/// `needs_you_api` operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NeedsYouOp {
    Ask,
    Show,
    Answer,
    Reconcile,
    Complete,
    ClaudeHook,
}

impl NeedsYouOp {
    pub(crate) fn parse(op: &str) -> Option<Self> {
        Some(match op {
            "ask" => Self::Ask,
            "show" => Self::Show,
            "answer" => Self::Answer,
            "reconcile" => Self::Reconcile,
            "complete" => Self::Complete,
            "claude_hook" => Self::ClaudeHook,
            _ => return None,
        })
    }
}

/// `agents_api` operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentsOp {
    CreateAgent,
    ListAgents,
    ListGrants,
    SpawnRun,
    ListRuns,
    ShowRun,
    FinishRun,
    Delegate,
    Call,
    TeamRun,
}

impl AgentsOp {
    pub(crate) fn parse(op: &str) -> Option<Self> {
        Some(match op {
            "create_agent" => Self::CreateAgent,
            "list_agents" => Self::ListAgents,
            "list_grants" => Self::ListGrants,
            "spawn_run" => Self::SpawnRun,
            "list_runs" => Self::ListRuns,
            "show_run" => Self::ShowRun,
            "finish_run" => Self::FinishRun,
            "delegate" => Self::Delegate,
            "call" => Self::Call,
            "team_run" => Self::TeamRun,
            _ => return None,
        })
    }
}

/// `agents dashboard` operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardOp {
    Open,
    Cancel,
    List,
}

impl DashboardOp {
    pub(crate) fn parse(op: &str) -> Option<Self> {
        Some(match op {
            "open" => Self::Open,
            "cancel" => Self::Cancel,
            "list" => Self::List,
            _ => return None,
        })
    }
}

/// Streaming frames served on the socket thread, outside `HostRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamFrame {
    CliTrace,
    EventsList,
    EventsSubscribe,
    EventsDeclare,
    EventsEmit,
    PaneLifecycleWait,
    PaneLifecycleFollow,
    CommandViewFollow,
}

impl StreamFrame {
    pub(crate) fn parse(kind: &str) -> Option<Self> {
        Some(match kind {
            "cli_trace" => Self::CliTrace,
            "events_list" => Self::EventsList,
            "events_subscribe" => Self::EventsSubscribe,
            "events_declare" => Self::EventsDeclare,
            "events_emit" => Self::EventsEmit,
            "pane_lifecycle_wait" => Self::PaneLifecycleWait,
            "pane_lifecycle_follow" => Self::PaneLifecycleFollow,
            "command_view_follow" => Self::CommandViewFollow,
            _ => return None,
        })
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CliTrace => "cli_trace",
            Self::EventsList => "events_list",
            Self::EventsSubscribe => "events_subscribe",
            Self::EventsDeclare => "events_declare",
            Self::EventsEmit => "events_emit",
            Self::PaneLifecycleWait => "pane_lifecycle_wait",
            Self::PaneLifecycleFollow => "pane_lifecycle_follow",
            Self::CommandViewFollow => "command_view_follow",
        }
    }

    pub(crate) fn policy(self) -> VerbPolicy {
        match self {
            Self::CliTrace
            | Self::EventsList
            | Self::PaneLifecycleWait
            | Self::PaneLifecycleFollow => VerbPolicy::Operator,
            Self::EventsSubscribe
            | Self::EventsDeclare
            | Self::EventsEmit
            | Self::CommandViewFollow => VerbPolicy::Protected,
        }
    }
}

/// A socket request refused before any handler ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl Refusal {
    pub(crate) fn reply(&self) -> serde_json::Value {
        // `status` for the clients that read a status rather than `ok`
        // (`plexi secret get`).
        serde_json::json!({
            "ok": false,
            "status": "denied",
            "error_code": self.code,
            "error": self.message,
        })
    }
}

/// Every Assistant host tool the socket can name, each with its own policy.
/// An id not listed here is refused before any handler runs.
pub(crate) const ASSISTANT_TOOLS: &[(&str, VerbPolicy)] = &[
    ("host.editors.list", VerbPolicy::Protected),
    ("host.introspect", VerbPolicy::Protected),
    ("host.permissions.list", VerbPolicy::Protected),
    ("host.permissions.revoke", VerbPolicy::Protected),
    ("host.permissions.reset", VerbPolicy::Widening),
    ("host.permissions.allow", VerbPolicy::Widening),
    ("host.panes.list", VerbPolicy::Protected),
    ("host.panes.state", VerbPolicy::Protected),
    ("host.panes.open", VerbPolicy::Protected),
    ("host.panes.focus", VerbPolicy::Protected),
    ("host.panes.close", VerbPolicy::Protected),
    ("host.views.open", VerbPolicy::Protected),
    ("host.terminals.open", VerbPolicy::Protected),
    ("host.terminals.run", VerbPolicy::Protected),
    ("host.terminals.read", VerbPolicy::Protected),
    ("host.files.read", VerbPolicy::Protected),
    ("host.files.grep", VerbPolicy::Protected),
    ("host.files.list", VerbPolicy::Protected),
    ("host.files.write", VerbPolicy::Protected),
    ("host.files.edit", VerbPolicy::Protected),
];

fn invalid_op(kind: &str, op: &str) -> Refusal {
    Refusal {
        code: "invalid_argument",
        message: format!("unknown {kind} operation '{op}'"),
    }
}

/// The policy of one socket request, decoding multiplexed suboperations.
/// An unknown suboperation is refused here, never dispatched.
pub(crate) fn socket_policy(request: &HostRequest) -> Result<VerbPolicy, Refusal> {
    use VerbPolicy::*;
    Ok(match request {
        // Stop is served to every same-user caller: an older client reads
        // any refusal as zero grace and kills the host (Codex 3290 r3).
        HostRequest::GetBuildInfo { .. }
        | HostRequest::Wake
        | HostRequest::Shutdown { .. }
        | HostRequest::ShutdownForRestart { .. } => Open,
        // Its handler answers every caller class with a typed receipt
        // (`caller_unidentified`, `no_anchor`, `outside_authority`).
        HostRequest::FileOpen { .. } => Open,

        HostRequest::ShowHost { .. }
        | HostRequest::SpawnReceipt { .. }
        | HostRequest::ContextRename { .. }
        | HostRequest::ContextClose { .. }
        | HostRequest::ContextPark { .. }
        | HostRequest::PaneZoom { .. }
        | HostRequest::PaneSwap { .. }
        | HostRequest::WorkspaceSnapshot { .. }
        | HostRequest::ReloadConfig { .. }
        | HostRequest::FocusHistory { .. }
        | HostRequest::ResourceLease { .. }
        | HostRequest::Notify { .. }
        | HostRequest::DismissNotification { .. }
        | HostRequest::SetPipStatus { .. }
        | HostRequest::GetAgentStates { .. }
        | HostRequest::LogMarker { .. }
        | HostRequest::SpawnPane { .. }
        | HostRequest::SetPaneTitle { .. }
        | HostRequest::ListPanes { .. }
        | HostRequest::ListContexts { .. }
        | HostRequest::GetPaneInfo { .. }
        | HostRequest::GetPreviousPaneInfo { .. }
        | HostRequest::SlotWrite { .. }
        | HostRequest::SlotRead { .. }
        | HostRequest::SlotWait { .. }
        | HostRequest::SlotList { .. }
        | HostRequest::SlotDelete { .. }
        | HostRequest::WorkspaceCleanSlots { .. }
        | HostRequest::PaneMessageSend { .. }
        | HostRequest::PaneMessageRecv { .. }
        | HostRequest::PaneResultSchemaSet { .. }
        | HostRequest::PaneResultWrite { .. }
        | HostRequest::PaneResultRead { .. }
        | HostRequest::AgentSpawn { .. }
        | HostRequest::AgentWait { .. }
        | HostRequest::AgentNext { .. }
        | HostRequest::AgentFleet { .. }
        | HostRequest::AgentFleetDismiss { .. }
        | HostRequest::AgentOpen { .. }
        | HostRequest::AgentNewChat { .. }
        | HostRequest::AgentReopen { .. }
        | HostRequest::FocusPane { .. }
        | HostRequest::ClosePane { .. }
        | HostRequest::PaneHeartbeat { .. }
        | HostRequest::KeyPane { .. }
        | HostRequest::DropFile { .. }
        | HostRequest::ClickPane { .. }
        | HostRequest::ClickPaneNode { .. }
        | HostRequest::DragPane { .. }
        | HostRequest::CapturePane { .. }
        | HostRequest::PaneStatus { .. }
        | HostRequest::GetPaneState { .. }
        | HostRequest::Screenshot { .. }
        | HostRequest::HostKey { .. }
        | HostRequest::SendPaneAction { .. }
        | HostRequest::ListNeedsYou { .. }
        | HostRequest::CallerContext { .. }
        | HostRequest::CreateContext { .. }
        | HostRequest::CreateSubContext { .. }
        | HostRequest::FocusContext { .. }
        | HostRequest::SetContextDescription { .. }
        | HostRequest::ZoomIntoContext { .. }
        | HostRequest::ZoomOutOfContext
        | HostRequest::PushPaneToSubcontext { .. } => Operator,

        HostRequest::PaneSecretList { .. }
        | HostRequest::SecretGet { .. }
        | HostRequest::SecretSet { .. }
        | HostRequest::SecretDelete { .. }
        | HostRequest::FolderSecretExec { .. }
        | HostRequest::RunSecrets { .. }
        | HostRequest::FolderSecretRead { .. }
        | HostRequest::FolderSecretSet { .. }
        | HostRequest::FolderSecretRm { .. }
        | HostRequest::CallPluginTool { .. }
        | HostRequest::ListPluginTools { .. }
        | HostRequest::SetContextRoot { .. }
        | HostRequest::AgentCall { .. }
        | HostRequest::AgentMessage { .. }
        | HostRequest::SubmitAssistantTurn { .. }
        | HostRequest::ListPermissions { .. }
        | HostRequest::ListPermissionRequests { .. }
        | HostRequest::ShowPermissionRequest { .. }
        | HostRequest::SendToPane { .. } => Protected,

        HostRequest::AssistantHostTool { name, .. } => ASSISTANT_TOOLS
            .iter()
            .find(|(tool, _)| tool == name)
            .map(|(_, policy)| *policy)
            .ok_or_else(|| invalid_op("assistant tool", name))?,
        HostRequest::Permissions { op, .. } => match PermissionsOp::parse(op) {
            Some(PermissionsOp::List | PermissionsOp::Revoke) => Protected,
            Some(PermissionsOp::Allow | PermissionsOp::Reset) => Widening,
            None => return Err(invalid_op("permissions", op)),
        },
        HostRequest::NeedsYouApi { op, .. } => match NeedsYouOp::parse(op) {
            Some(
                NeedsYouOp::Ask | NeedsYouOp::Show | NeedsYouOp::Reconcile | NeedsYouOp::Complete,
            ) => Protected,
            Some(NeedsYouOp::Answer) => Decision,
            Some(NeedsYouOp::ClaudeHook) => OwnPane,
            None => return Err(invalid_op("needs_you", op)),
        },
        HostRequest::AgentsApi { op, .. } => match AgentsOp::parse(op) {
            Some(
                AgentsOp::ListAgents
                | AgentsOp::ListGrants
                | AgentsOp::ListRuns
                | AgentsOp::ShowRun
                | AgentsOp::FinishRun
                | AgentsOp::SpawnRun
                | AgentsOp::Call,
            ) => Protected,
            Some(AgentsOp::CreateAgent | AgentsOp::Delegate | AgentsOp::TeamRun) => Widening,
            None => return Err(invalid_op("agents", op)),
        },
        HostRequest::AgentsDashboard { op, .. } => match DashboardOp::parse(op) {
            Some(DashboardOp::Open | DashboardOp::List | DashboardOp::Cancel) => Protected,
            None => return Err(invalid_op("agents dashboard", op)),
        },

        HostRequest::ResolvePermissionRequest { .. } | HostRequest::ResolveNeedsYou { .. } => {
            Decision
        }

        HostRequest::SetAgentState { .. }
        | HostRequest::AgentDone { .. }
        | HostRequest::AgentAsk { .. } => OwnPane,

        HostRequest::CapabilityRequest { .. }
        | HostRequest::PluginSecretGet { .. }
        | HostRequest::FileRead { .. }
        | HostRequest::FileList { .. }
        | HostRequest::SavePluginState { .. }
        | HostRequest::RunGet { .. }
        | HostRequest::RunComplete { .. }
        | HostRequest::PipeOpen { .. }
        | HostRequest::PipeOpenDirected { .. }
        | HostRequest::PipeSend { .. }
        | HostRequest::StatusSummary { .. }
        | HostRequest::SpawnView { .. }
        | HostRequest::QueryContextState { .. }
        | HostRequest::HttpRequest { .. }
        | HostRequest::OpenUrl { .. }
        | HostRequest::AiQuery { .. }
        | HostRequest::ExposeTools { .. }
        | HostRequest::ToolResult { .. }
        | HostRequest::AudioPlay { .. }
        | HostRequest::AudioCapture { .. }
        | HostRequest::ListAudioDevices { .. }
        | HostRequest::ListMidiDevices { .. }
        | HostRequest::OpenMidiInput { .. }
        | HostRequest::CloseMidiInput { .. }
        | HostRequest::SendMidi { .. }
        | HostRequest::OpenVideo { .. }
        | HostRequest::SetVideoState { .. }
        | HostRequest::CloseVideo { .. }
        | HostRequest::CdRequest { .. }
        | HostRequest::SetTimer { .. }
        | HostRequest::CancelTimer { .. }
        | HostRequest::LoadImage { .. }
        | HostRequest::RequestLinkedTerminal { .. }
        | HostRequest::RunInLinkedTerminal { .. }
        | HostRequest::InsertPathToken { .. }
        | HostRequest::RequestCommandPreview { .. }
        | HostRequest::OpenArtifact { .. }
        | HostRequest::PushNav { .. }
        | HostRequest::PopNav { .. }
        | HostRequest::SetMouseTracking { .. }
        | HostRequest::StreamProcess { .. }
        | HostRequest::CancelProcess { .. }
        | HostRequest::OpenFilePicker { .. }
        | HostRequest::DeclareEventStreams { .. }
        | HostRequest::EmitEvent { .. }
        | HostRequest::RollbackVerifyResult { .. }
        | HostRequest::SubscribePluginEvents { .. }
        | HostRequest::UnsubscribePluginEvents { .. }
        | HostRequest::ListUndoCheckpoints { .. }
        | HostRequest::RequestRollback { .. } => NotOnSocket,
    })
}

/// Whether `caller` may reach a handler of class `policy`.
pub(crate) fn admit_request(
    caller: &SocketCaller,
    request: &HostRequest,
    policy: VerbPolicy,
) -> Result<(), Refusal> {
    // A secret handler brokers detached callers as External and requires a
    // fresh one-use grant. Other protected handlers require an identified peer.
    if matches!(caller, SocketCaller::Unidentified(_))
        && matches!(
            request,
            HostRequest::SecretGet { .. }
                | HostRequest::SecretSet { .. }
                | HostRequest::SecretDelete { .. }
                | HostRequest::FolderSecretExec { .. }
                | HostRequest::RunSecrets { .. }
                | HostRequest::FolderSecretRead { .. }
                | HostRequest::FolderSecretSet { .. }
                | HostRequest::FolderSecretRm { .. }
        )
    {
        return Ok(());
    }
    admit(caller, policy)
}

pub(crate) fn admit(caller: &SocketCaller, policy: VerbPolicy) -> Result<(), Refusal> {
    let unidentified = |reason: String| Refusal {
        code: "permission_denied",
        message: format!(
            "caller identity could not be established: {reason}. Run the command from a Plexi pane or a terminal outside Plexi."
        ),
    };
    match (policy, caller) {
        (VerbPolicy::NotOnSocket, _) => Err(Refusal {
            code: "not_on_socket",
            message: "this request is for plugins and is not served on the command socket"
                .to_string(),
        }),
        (VerbPolicy::Decision, _) => Err(Refusal {
            code: "approval_required",
            message: "Approve it in the Plexi window.".to_string(),
        }),
        // Pane driving and observation is attribution, not a boundary, for
        // every same-user caller (the socket is the user's own); a walk
        // that could not be completed still drives a pane, as on alpha
        // (Codex 3290 r3).
        (VerbPolicy::Open, _) | (VerbPolicy::Operator, SocketCaller::Unidentified(_)) => Ok(()),
        (
            VerbPolicy::Operator | VerbPolicy::Protected | VerbPolicy::Widening,
            SocketCaller::Pane(_) | SocketCaller::External(_),
        ) => Ok(()),
        (VerbPolicy::OwnPane, SocketCaller::Pane(_)) => Ok(()),
        (VerbPolicy::OwnPane, SocketCaller::External(_)) => Err(Refusal {
            code: "permission_denied",
            message: "only a process running in a Plexi pane can make this request".to_string(),
        }),
        (
            VerbPolicy::Protected | VerbPolicy::Widening | VerbPolicy::OwnPane,
            SocketCaller::Unidentified(reason),
        ) => Err(unidentified(reason.to_string())),
    }
}

/// The one info trace for every request the entry does not dispatch.
pub(crate) fn trace_refusal(operation: &str, caller: &SocketCaller, policy: Option<VerbPolicy>, refusal: &Refusal) {
    log::info!(
        "socket_policy: refused operation={operation} policy={} caller={} principal={} outcome={}",
        policy.map_or("unclassified", VerbPolicy::as_str),
        caller.class(),
        caller
            .principal()
            .map_or_else(|| "none".to_string(), |p| p.to_string()),
        refusal.code
    );
}

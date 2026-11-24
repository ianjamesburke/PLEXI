//! Agent tool calls use the worker lifecycle and the host's structured mailbox.
use super::PlexiApp;
use crate::broker::caller::{RequestOrigin, SocketCaller};
use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor, UseDecision};
use crate::broker::permit::{
    self, ContextRootProof, Coverage, Operation, Permit, ReadDashboard, SetContextRoot,
};
use crate::broker::principal::Principal;
use crate::broker::PermissionPosture;
use crate::host::context::PaneCwd;
use crate::host::plugin_timeline::AdmittedScope;
use crate::protocol::AgentState;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The caller of an agent tool, as the host established it (stint 1285).
pub(crate) struct AgentCaller<'a> {
    pub origin: &'a crate::broker::caller::RequestOrigin,
    pub credential: Option<&'a str>,
}

pub(crate) struct AgentCallRequest<'a> {
    pub role: &'a str,
    pub task: &'a str,
    pub origin: &'a crate::broker::caller::RequestOrigin,
    pub credential: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub worktree: Option<&'a str>,
    pub asynchronous: bool,
    pub handoff: bool,
    pub response_file: &'a str,
}

pub(crate) struct AgentToolWait {
    pub pane_id: u64,
    response_file: String,
    deadline: Instant,
    resolve_idle: bool,
}

/// The incarnation that supplied a value, or the shared outside-process principal.
pub(super) enum UseSubject {
    Pane(AdmittedScope),
    External,
}

struct CallerUse {
    monitor: Arc<PermissionMonitor>,
    actor: String,
    call: String,
}

impl PlexiApp {
    /// The pane an agent tool call acts for. An outside process acts for no
    /// pane and each use goes through [`Self::admit_outside_use`]; an
    /// unidentified caller acts for nobody; host code acts unparented
    /// (stint 1285).
    pub(crate) fn agent_tool_caller(&self, caller: AgentCaller<'_>) -> Result<Option<u64>, String> {
        use crate::broker::caller::{RequestOrigin, SocketCaller};
        let socket = match caller.origin {
            RequestOrigin::Socket(socket) => socket,
            RequestOrigin::HostUi | RequestOrigin::HostSystem => return Ok(None),
            other => {
                return Err(format!(
                    "permission_denied: agent tools are not served to {}",
                    other.class()
                ))
            }
        };
        if let SocketCaller::External(_) = socket {
            return Ok(None);
        }
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .map_err(|error| error.to_string())?;
        let verified = monitor
            .authenticate_call(socket, caller.credential)
            .map_err(|e| format!("permission_denied: caller identity: {e:?}"))?;
        if let Some(id) = verified.pane_id {
            if self.find_pane_in_any_window(id).is_none() {
                return Err(format!("caller pane {id} is no longer live"));
            }
        }
        Ok(verified.pane_id)
    }

    /// Admit one use by an outside process through the broker. Its identity
    /// is shared by every process outside Plexi, so a grant lasts once or for
    /// the session and an approval names this exact input. Any other origin
    /// passes: it was admitted as itself.
    pub(crate) fn admit_outside_use(
        &self,
        origin: &crate::broker::caller::RequestOrigin,
        tool: &str,
        input: &Value,
        root: &std::path::Path,
        context_id: u64,
    ) -> Result<(), String> {
        use crate::broker::caller::{RequestOrigin, SocketCaller};
        if !matches!(origin, RequestOrigin::Socket(SocketCaller::External(_))) {
            return Ok(());
        }
        self.admit_caller_use(origin, tool, input, root, context_id)
    }

    /// Admit one use by a socket caller through the broker as its own
    /// principal: a pane under its ceiling, an outside process under the
    /// shared outside identity (once or session). An unidentified caller is
    /// refused; host code passes, it was admitted as itself (stint 1285).
    pub(crate) fn admit_caller_use(
        &self,
        origin: &crate::broker::caller::RequestOrigin,
        tool: &str,
        input: &Value,
        root: &std::path::Path,
        context_id: u64,
    ) -> Result<(), String> {
        if matches!(origin, RequestOrigin::HostUi | RequestOrigin::HostSystem) {
            return Ok(());
        }
        let subject = UseSubject::from_origin(origin)?;
        let decide = PermissionMonitor::admit_with_posture;
        let (caller, answer) = subject.decide(tool, input, root, context_id, decide)?;
        caller.finish_immediate(tool, answer)
    }
}

impl UseSubject {
    pub(super) fn from_origin(origin: &RequestOrigin) -> Result<Self, String> {
        match origin.socket_caller() {
            Some(SocketCaller::Pane(pane)) => {
                let scope = AdmittedScope::of(&pane.authority);
                if scope.authority.scope().context_id != pane.context_id {
                    return Err("permission_denied: caller pane changed context".into());
                }
                Ok(Self::Pane(scope))
            }
            Some(SocketCaller::External(_)) => Ok(Self::External),
            Some(SocketCaller::Unidentified(reason)) => Err(format!(
                "permission_denied: caller identity could not be established: {reason}"
            )),
            None => Err(format!(
                "permission_denied: {} has no socket caller proof",
                origin.class()
            )),
        }
    }

    fn decide<D>(
        &self,
        tool: &str,
        input: &Value,
        root: &Path,
        context_id: u64,
        decide: impl FnOnce(&PermissionMonitor, AdmitRequest<'_>, Option<&PermissionPosture>) -> D,
    ) -> Result<(CallerUse, D), String> {
        let (principal, posture) = match self {
            Self::Pane(scope) => {
                let live = scope.current()?;
                let actor = Principal::agent(format!("pane:{}", live.pane_id()));
                let posture = live.grant().posture().map_err(|e| e.to_string())?;
                (actor, posture)
            }
            Self::External => (Principal::External, None),
        };
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir())
            .map_err(|e| e.to_string())?;
        let actor = principal.actor_id().to_string();
        let call = format!("caller-{tool}-{}", uuid::Uuid::new_v4());
        let request = AdmitRequest {
            call_id: &call,
            tool,
            input_json: &input.to_string(),
            principal: &principal,
            actor_scope: crate::broker::ActorScope::User,
            trust_origin: "plexi-cli",
            workspace_root: &crate::platform::path::canonical_or_self(root),
            context_id,
            package_id: "",
            instance_id: 0,
            target_type: crate::broker::TargetType::HostTool,
        };
        let answer = decide(&monitor, request, posture);
        Ok((
            CallerUse {
                monitor,
                actor,
                call,
            },
            answer,
        ))
    }
}

impl CallerUse {
    fn note_use(&self, grant: &str, fingerprint: &str, resource: &str) -> Result<(), String> {
        self.monitor
            .note_use(&self.actor, &self.call, grant, fingerprint, resource, "")
    }

    fn outcome(
        &self,
        grant: &str,
        fingerprint: &str,
        resource: &str,
        outcome: &str,
    ) -> Result<(), String> {
        self.monitor.note_outcome(
            &self.actor,
            &self.call,
            grant,
            fingerprint,
            resource,
            "",
            outcome,
            "",
            "",
        )
    }

    fn finish_immediate(&self, tool: &str, answer: Admission) -> Result<(), String> {
        let actor = &self.actor;
        match answer {
            Admission::Proceed {
                grant_id,
                fingerprint,
                resource_id,
                ..
            } => {
                let resource = resource_id.unwrap_or_default();
                self.note_use(&grant_id, &fingerprint, &resource)?;
                self.outcome(&grant_id, &fingerprint, &resource, "ok")?;
                self.monitor.consume_once(&grant_id, None);
                log::info!("caller_use: admitted tool={tool} actor={actor} grant={grant_id}");
                Ok(())
            }
            Admission::Required { pending_request_id } => {
                log::info!("caller_use: tool={tool} actor={actor} waits on {pending_request_id}");
                Err(needs_approval(tool, actor, &pending_request_id))
            }
            Admission::Denied { code } => {
                log::info!("caller_use: tool={tool} actor={actor} denied code={code}");
                Err(format!("permission_denied: {tool} for {actor} was denied ({code})"))
            }
        }
    }
}

impl UseSubject {
    pub(super) fn admit_root_change(
        &self,
        workspace: &Path,
        id: u64,
        root: &Path,
    ) -> Result<(Permit<SetContextRoot>, impl FnOnce(bool)), String> {
        let (canonical, target) = SetContextRoot::target(id, root)?;
        let tool = SetContextRoot::NAME;
        let input = json!({"context_id": id, "root": canonical});
        let decide = PermissionMonitor::admit_for_use;
        let (caller, answer) = self.decide(tool, &input, workspace, id, decide)?;
        let actor = &caller.actor;
        let (approved, grant, fingerprint, resource) = match answer {
            UseDecision::Allowed {
                approved,
                grant_id,
                fingerprint,
                resource_id,
            } => (*approved, grant_id, fingerprint, resource_id),
            UseDecision::Required { pending_request_id } => {
                return Err(needs_approval(tool, actor, &pending_request_id))
            }
            UseDecision::Denied => {
                return Err(format!("permission_denied: {tool} for {actor} was denied"))
            }
        };
        let coverage = Coverage::new(caller.monitor.clone(), vec![(target.clone(), approved)]);
        let permit = match self {
            Self::Pane(scope) => permit::mint_pane(scope.clone(), target, coverage)?,
            Self::External => permit::mint_unbound("external", target, coverage),
        };
        caller.note_use(&grant, &fingerprint, &resource)?;
        // Audit after the irreversible handoff, outside the admission lock.
        let record_outcome = move |applied| {
            let outcome = if applied { "ok" } else { "denied" };
            if let Err(error) = caller.outcome(&grant, &fingerprint, &resource, outcome) {
                log::error!("context.set_root: could not record {outcome}: {error}");
            }
        };
        Ok((permit, record_outcome))
    }
}

impl UseSubject {
    /// Admit this subject to read `workspace`'s dashboard. A pane's
    /// containment is the caller's check and the permit pins the incarnation
    /// it was checked under; an outside process needs an approved use of its
    /// own for exactly this workspace.
    pub(super) fn admit_dashboard_read(
        &self,
        workspace: &Path,
    ) -> Result<Permit<ReadDashboard>, String> {
        let Self::Pane(scope) = self else {
            return self.admit_external_dashboard_read(workspace);
        };
        scope.current()?;
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir())
            .map_err(|e| e.to_string())?;
        Permit::contained(scope.clone(), workspace, monitor)
    }

    fn admit_external_dashboard_read(&self, workspace: &Path) -> Result<Permit<ReadDashboard>, String> {
        let tool = ReadDashboard::NAME;
        let target = ReadDashboard::target(workspace);
        let input = json!({"workspace": target});
        let decide = PermissionMonitor::admit_for_use;
        let (caller, answer) = self.decide(tool, &input, Path::new(&target), 0, decide)?;
        let (approved, grant, fingerprint, resource) = match answer {
            UseDecision::Allowed {
                approved,
                grant_id,
                fingerprint,
                resource_id,
            } => (*approved, grant_id, fingerprint, resource_id),
            UseDecision::Required { pending_request_id } => {
                return Err(needs_approval(tool, &caller.actor, &pending_request_id))
            }
            UseDecision::Denied => {
                return Err(format!(
                    "permission_denied: {tool} for {} was denied",
                    caller.actor
                ))
            }
        };
        caller.note_use(&grant, &fingerprint, &resource)?;
        caller.outcome(&grant, &fingerprint, &resource, "ok")?;
        Ok(Permit::external(workspace, caller.monitor, approved))
    }
}

fn needs_approval(tool: &str, actor: &str, pending_request_id: &str) -> String {
    format!(
        "permission_denied: {tool} needs approval for {actor}. \
             Approve request {pending_request_id} in Needs you, then rerun this command."
    )
}

impl PlexiApp {
    pub(super) fn set_root_for(
        &mut self,
        subject: UseSubject,
        root: &Path,
        id: u64,
    ) -> Result<(), String> {
        let idx = self.resolve_context_idx(Some(id), "set_context_root")?;
        let (permit, record_outcome) =
            subject.admit_root_change(&self.router.get(idx).root, id, root)?;
        let result = self.commit_context_root(root.into(), id, ContextRootProof::Permitted(permit));
        record_outcome(result.is_ok());
        result
    }

    /// Adopt the focused pane's root. A person's own gesture applies it in one
    /// step; input marked synthetic (socket-injected) is the pane's own
    /// request and needs the pane's permit.
    pub(crate) fn set_root_from_pane(&mut self, source: PaneCwd) {
        let id = source.target_context_id;
        let result = if self.synthetic_input_frame {
            source
                .scope
                .ok_or_else(|| "synthetic input without a terminal authority".to_string())
                .and_then(|scope| self.set_root_for(UseSubject::Pane(scope), &source.cwd, id))
        } else {
            self.commit_context_root(source.cwd, id, crate::overlays::misc::window_chosen_root())
        };
        if let Err(error) = result {
            log::info!("set_root_from_pane: context {id} refused: {error}");
        }
    }

    /// The caller's `can_call`/`can_handoff` grant, read under the admitted
    /// origin context's root. The origin is explicit: an outside caller is
    /// never re-resolved against whatever window is active now (stint 1632).
    pub(crate) fn check_agent_tool_permission(
        &self,
        caller: Option<u64>,
        origin_context_id: u64,
        role: &str,
        handoff: bool,
    ) -> Result<std::path::PathBuf, String> {
        if let Some(id) = caller {
            let (win, _) = self
                .find_pane_in_any_window(id)
                .ok_or_else(|| format!("caller pane {id} not found"))?;
            let now = self.windows[win].context_id;
            if now != origin_context_id {
                return Err(format!(
                    "caller pane {id} is in context {now}, not its admitted context {origin_context_id}"
                ));
            }
        }
        let root = self
            .router
            .iter()
            .find(|c| c.context_id == origin_context_id)
            .map(|c| c.root.clone())
            .ok_or_else(|| format!("origin context {origin_context_id} no longer exists"))?;
        // The caller's delegation limit is fixed in its grant (stint 1632):
        // a plain shell carries the limits it inherited with no role of its
        // own, and a role file edited later cannot widen a running pane.
        if let Some(id) = caller {
            let authority = self
                .pane_authority(id)
                .ok_or_else(|| format!("permission_denied: caller pane {id} has no grant"))?;
            let grant = authority.grant();
            grant.posture().map_err(|sealed| sealed.to_string())?;
            if !grant.delegation(handoff).allows(role) {
                let field = if handoff { "can_handoff" } else { "can_call" };
                log::info!(
                    "agent_call: pane {id} (role {:?}) may not delegate '{role}' ({field})",
                    grant.role
                );
                return Err(format!(
                    "permission_denied: pane {id} may not delegate to '{role}': not in its {field} grant"
                ));
            }
        }
        Ok(root)
    }

    pub(crate) fn start_agent_tool_call(
        &mut self,
        request: AgentCallRequest<'_>,
    ) -> Result<(), String> {
        super::agent_open::admit_agent_mesh_route("AgentCall")?;
        let plan = self.admit_agent_call(&request)?;
        let child_parent = plan.child_parent;
        let parent = plan.authority.parent;
        let (role, task) = (plan.authority.role.clone(), plan.task.clone());
        // `apply_launch` owns any checkout from preparation to the pane, and
        // reclaims or retains it on refusal (stint 1632).
        let pane_id = self
            .apply_launch(super::launch_plan::LaunchPlan::Agent(Box::new(plan)))
            .map_err(|refused| refused.to_string())?;
        self.agent_tool_children.insert(pane_id, child_parent);
        log::info!("agent_call: spawned role={role} parent={child_parent:?} child={pane_id} async={} handoff={}", request.asynchronous, request.handoff);
        if let (true, Some(old)) = (request.handoff, parent) {
            // The caller's completion names its successor and is published
            // before the caller closes; the reply carries both ids.
            let raw = super::launch_plan::handoff_completion(&role, &task, pane_id);
            let summary = format!("Handed off to {role}");
            let mut reply = self.complete_agent_pane(old, Some(&raw), Some(&summary));
            if reply["ok"] != true {
                log::warn!(
                    "agent_handoff: completion refused; closing successor={pane_id}: {reply}"
                );
                self.reclaim_pane_worktree(pane_id);
                self.close_pane_by_id(pane_id);
                return Err(format!(
                    "handoff completion: {}",
                    reply["error"].as_str().unwrap_or("rejected")
                ));
            }
            reply["successor_pane_id"] = json!(pane_id);
            crate::rpc::write_json_response(request.response_file, reply);
            // Handoff transfers ownership, independently of ordinary done's
            // optional ephemeral cleanup.
            if self.find_pane_in_any_window(old).is_some() {
                self.close_pane_by_id(old);
            }
            log::info!("agent_handoff: closed caller={old} successor={pane_id}");
        } else if request.asynchronous {
            crate::rpc::write_json_response(
                request.response_file,
                json!({"ok":true,"status":"working","pane_id":pane_id}),
            );
        } else {
            self.park_agent_tool_wait(pane_id, request.response_file);
        }
        Ok(())
    }

    pub(crate) fn park_agent_tool_wait(&mut self, pane_id: u64, response_file: &str) {
        self.agent_tool_waits.push(AgentToolWait {
            pane_id,
            response_file: response_file.into(),
            deadline: Instant::now() + Duration::from_secs(86400),
            resolve_idle: false,
        });
    }

    pub(crate) fn send_agent_tool_message(
        &mut self,
        pane_id: u64,
        text: &str,
        caller: AgentCaller<'_>,
        wait: bool,
        response_file: &str,
    ) -> Result<(), String> {
        let origin = caller.origin.clone();
        let parent = self.agent_tool_caller(caller)?;
        let (win, _) = self
            .find_pane_in_any_window(pane_id)
            .ok_or_else(|| format!("pane {pane_id} not found"))?;
        if !text.trim().is_empty() {
            let context_id = self.windows[win].context_id;
            let root = self.windows[win].path.clone();
            self.admit_outside_use(
                &origin,
                "agent.message",
                &json!({"pane_id": pane_id, "text": text}),
                &root,
                context_id,
            )?;
        }
        let terminal = self.windows[win]
            .panes
            .get_mut(&pane_id)
            .and_then(|p| p.as_terminal_mut())
            .ok_or("target is not an agent terminal")?;
        if parent.is_some() && terminal.parent_pane_id != parent {
            return Err("permission_denied: target is not your child agent".into());
        }
        if text.trim().is_empty() {
            return Err("message must be non-empty".into());
        }
        if terminal.agent_completed {
            return Err("agent already completed".into());
        }
        terminal.last_blocked_reason = None;
        if let Some(state) = terminal.agent.as_mut() {
            state.state = AgentState::Working;
            state.detail = None;
        }
        let (message_id, _) = self.enqueue_pane_message(pane_id, parent, text.into());
        log::info!(
            "agent_message: parent={parent:?} child={pane_id} message_id={message_id} wait={wait}"
        );
        if wait {
            self.park_agent_tool_wait(pane_id, response_file);
            if let Some(waiter) = self.agent_tool_waits.last_mut() {
                waiter.resolve_idle = true;
            }
        } else {
            crate::rpc::write_json_response(
                response_file,
                json!({"ok":true,"pane_id":pane_id,"message_id":message_id}),
            );
        }
        Ok(())
    }

    pub(crate) fn finish_agent_tool_waits(&mut self, pane_id: u64, outcome: Value) {
        let (done, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.agent_tool_waits)
            .into_iter()
            .partition(|w| w.pane_id == pane_id);
        self.agent_tool_waits = live;
        for wait in done {
            crate::rpc::write_json_response(&wait.response_file, outcome.clone());
        }
    }

    pub(crate) fn service_agent_tool_waits(&mut self, ctx: &egui::Context) {
        // Rebuild attachment from persisted worker ancestry after session restore.
        // Portal-backed sub-contexts keep their original navigation model.
        for win in &self.windows {
            if self.conjoined_contexts.contains_key(&win.context_id) {
                continue;
            }
            if !self
                .router
                .iter()
                .any(|c| c.context_id == win.context_id && c.parent_id.is_some())
            {
                continue;
            }
            if self
                .windows
                .iter()
                .flat_map(|w| w.panes.values())
                .any(|p| p.portal_target() == Some(win.context_id))
            {
                continue;
            }
            if let Some(parent) = win
                .panes
                .values()
                .filter_map(|p| p.as_terminal().and_then(|t| t.parent_pane_id))
                .find(|id| {
                    self.find_pane_in_any_window(*id)
                        .is_some_and(|(index, _)| self.windows[index].context_id != win.context_id)
                })
            {
                self.conjoined_contexts.insert(win.context_id, parent);
                for (&id, pane) in win.panes.iter() {
                    if pane
                        .as_terminal()
                        .is_some_and(|t| t.parent_pane_id == Some(parent))
                    {
                        self.agent_tool_children.insert(id, Some(parent));
                    }
                }
                log::info!(
                    "agent_conjoined: restored parent={parent} context={}",
                    win.context_id
                );
            }
        }
        self.prune_conjoined_contexts();
        self.agent_tool_children
            .retain(|id, _| self.windows.iter().any(|w| w.panes.contains_key(id)));
        let now = Instant::now();
        let outcomes: Vec<_> = self.agent_tool_waits.iter().filter_map(|wait| {
            let pane = self.windows.iter().find_map(|w| w.panes.get(&wait.pane_id));
            let outcome = if now >= wait.deadline { Some(json!({"ok":false,"status":"timeout","error":"agent call timed out"})) }
            else if pane.is_none() { Some(json!({"ok":false,"status":"closed","error":"agent closed without completing"})) }
            else if wait.resolve_idle {
                match pane.and_then(|p| p.agent()) {
                    Some(agent) if agent.state == AgentState::Idle => Some(json!({"ok":true,"pane_id":wait.pane_id,"status":"idle"})),
                    Some(agent) if matches!(agent.state, AgentState::Blocked | AgentState::NeedsYou) => Some(json!({"ok":true,"pane_id":wait.pane_id,"status":"question","question":agent.detail})),
                    _ => None,
                }
            } else { None };
            outcome.map(|v| (wait.pane_id, v))
        }).collect();
        for (id, outcome) in outcomes {
            self.finish_agent_tool_waits(id, outcome);
        }
        if let Some(deadline) = self.agent_tool_waits.iter().map(|w| w.deadline).min() {
            ctx.request_repaint_after(
                deadline.saturating_duration_since(now) + Duration::from_millis(20),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::HostHarness;

    fn read(path: &std::path::Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn agent_tool_message_cli_process() {
        let Ok(args) = std::env::var("PLEXI_TEST_AGENT_MESSAGE_ARGS") else {
            return;
        };
        use clap::Parser;
        let args: Vec<String> = serde_json::from_str(&args).unwrap();
        let cli = crate::cli::args::Cli::parse_from(args);
        let Some(crate::cli::args::Commands::Agent {
            cmd:
                crate::cli::args::AgentCmd::Message {
                    pane_id,
                    text,
                    wait,
                },
        }) = cli.command
        else {
            panic!("expected agent message");
        };
        let pane_id = pane_id.resolve(crate::cli::OutputMode::Text).unwrap();
        std::process::exit(crate::cli::agent::agent_tool_cli(json!({
            "type":"agent_message", "pane_id":pane_id, "text":text, "wait":wait,
        })));
    }

    #[cfg(unix)]
    #[test]
    fn agent_tool_message_cli_socket_round_trip_and_wait() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixListener;
        use std::process::{Command, Stdio};
        let mut h = HostHarness::new();
        let parent = h.add_focused_terminal();
        let (child, _) = h
            .app
            .spawn_agent_worker_pane(
                "tester",
                "pi",
                "sleep 30",
                "Verify",
                None,
                false,
                Some(parent),
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let root = h.workspace_root();
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(parent),
            "agent:lead",


        );
        let socket_dir = tempfile::tempdir_in("/tmp").unwrap();
        let socket = socket_dir.path().join("message.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        for wait in [false, true] {
            let mut args = vec![
                "plexi".to_owned(),
                "agent".into(),
                "message".into(),
                child.to_string(),
                "reply with spaces".into(),
            ];
            if wait {
                args.push("--wait".into());
            }
            let mut process = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "app::agent_tools::tests::agent_tool_message_cli_process",
                    "--nocapture",
                    // Keep libtest's single-threaded name prefix off the JSON line.
                    "--quiet",
                ])
                .env(
                    "PLEXI_TEST_AGENT_MESSAGE_ARGS",
                    serde_json::to_string(&args).unwrap(),
                )
                .env("PLEXI_SOCKET", &socket)
                .env("PLEXI_PANE_ID", parent.to_string())
                .env("PLEXI_CALL_CREDENTIAL", &token)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline =
                Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
            let stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                if Instant::now() > deadline {
                    process.kill().unwrap();
                    process.wait().unwrap();
                    panic!("CLI did not connect");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut wire = String::new();
            reader.read_line(&mut wire).unwrap();
            // This is the production socket decoder, not direct typed IPC injection.
            let peer = [h.shell_pid(parent)];
            crate::app::handle_socket_line(&wire, &h.ipc_tx, Some(&peer), reader.get_mut());
            h.hidden_frame();
            let delivered = h
                .app
                .pane_mailboxes
                .get(&child)
                .and_then(|q| q.back())
                .map(|m| m.payload.as_str())
                == Some("reply with spaces");
            if !delivered {
                process.kill().unwrap();
                process.wait().unwrap();
                panic!("CLI wire request was not delivered: {wire}");
            }
            if wait {
                assert!(
                    process.try_wait().unwrap().is_none(),
                    "--wait returned before child outcome"
                );
                h.inject_ipc(crate::protocol::HostRequest::AgentAsk {
                    pane_id: child,
                    question: "Need anything else?".into(),
                    response_file: root.join("ask.json").to_string_lossy().into_owned(),
                });
                h.hidden_frame();
            }
            loop {
                if process.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() > deadline {
                    process.kill().unwrap();
                    process.wait().unwrap();
                    panic!("CLI did not receive its response");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let output = process.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            let response: Value =
                serde_json::from_str(stdout.lines().find(|l| l.starts_with('{')).unwrap()).unwrap();
            assert_eq!(response["ok"], true);
            if wait {
                assert_eq!(response["status"], "question");
                assert_eq!(response["question"], "Need anything else?");
            }
            h.app.pane_mailboxes.get_mut(&child).unwrap().clear();
        }
    }

    #[test]
    fn agent_tool_handoff_closes_non_ephemeral_spawned_caller() {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        for role in ["lead", "writer"] {
            let dir = crate::agent::workspace_agents_dir(&root).join(role);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("AGENT.md"), "Work carefully").unwrap();
            std::fs::write(dir.join("settings.toml"), format!("[agent]\nid='{role}'\ndisplay_name='{role}'\n[harness]\ncli='pi'\nephemeral=false\ncan_handoff=['writer']")).unwrap();
        }
        let (caller, _) = h
            .app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "sleep 30",
                "Start",
                None,
                false,
                None,
                Some(root.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        assert!(!h.app.agent_tool_children.contains_key(&caller));
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(caller),
            "agent:lead",


        );
        let response = root.join("handoff.json");
        h.app
            .start_agent_tool_call(AgentCallRequest {
                role: "writer",
                task: "Continue",
                origin: &h.origin_from_pane(caller),
                credential: Some(&token),
                cwd: None,
                worktree: None,
                asynchronous: false,
                handoff: true,
                response_file: response.to_str().unwrap(),
            })
            .unwrap();
        assert_eq!(read(&response)["ok"], true);
        assert!(h
            .app
            .windows
            .iter()
            .flat_map(|w| w.panes.values())
            .any(|p| p
                .as_terminal()
                .is_some_and(|t| t.agent_role.as_deref() == Some("writer"))));
        assert!(
            h.app.find_pane_in_any_window(caller).is_none(),
            "handoff retained its non-ephemeral caller"
        );
    }

    #[test]
    fn agent_tool_handoff_schema_failure_does_not_spawn_a_successor() {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        let parent = h.add_focused_terminal();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        for role in ["lead", "writer"] {
            let dir = crate::agent::workspace_agents_dir(&root).join(role);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("AGENT.md"), "Work carefully").unwrap();
            std::fs::write(dir.join("settings.toml"), format!("[agent]\nid='{role}'\ndisplay_name='{role}'\n[harness]\ncli='pi'\ncan_handoff=['writer']")).unwrap();
        }
        h.app.windows[0]
            .panes
            .get_mut(&parent)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .agent_role = Some("lead".into());
        h.app
            .pane_results
            .entry(parent)
            .or_default()
            .set_schema(json!({"type":"object","required":["verdict"]}), None)
            .unwrap();
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(parent),
            "agent:lead",


        );
        let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        let response = root.join("handoff.json");
        let error = h
            .app
            .start_agent_tool_call(AgentCallRequest {
                role: "writer",
                task: "Continue",
                origin: &h.origin_from_pane(parent),
                credential: Some(&token),
                cwd: None,
                worktree: None,
                asynchronous: false,
                handoff: true,
                response_file: response.to_str().unwrap(),
            })
            .unwrap_err();
        assert!(error.contains("handoff completion"), "{error}");
        assert_eq!(
            h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>(),
            before
        );
        assert!(h.app.pane_results[&parent].committed.is_none());
        assert!(h.app.find_pane_in_any_window(parent).is_some());
    }

    #[test]
    fn agent_tool_message_authenticates_parent_and_waits_for_next_idle() {
        let mut h = HostHarness::new();
        let parent = h.add_focused_terminal();
        let (child, _) = h
            .app
            .spawn_agent_worker_pane(
                "tester",
                "pi",
                "sleep 30",
                "Verify",
                None,
                false,
                Some(parent),
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let root = h.workspace_root();
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(parent),
            "agent:lead",


        );
        let reply = root.join("message.json");
        // A credential another pane holds proves nothing about this caller.
        let foreign = monitor.issue_credential(Some(child), "agent:other");
        assert!(h
            .app
            .send_agent_tool_message(
                child,
                "answer",
                AgentCaller {
                    origin: &h.origin_from_pane(parent),
                    credential: Some(&foreign),
                },
                true,
                reply.to_str().unwrap()
            )
            .is_err());
        h.app
            .send_agent_tool_message(
                child,
                "answer",
                AgentCaller {
                    origin: &h.origin_from_pane(parent),
                    credential: Some(&token),
                },
                true,
                reply.to_str().unwrap(),
            )
            .unwrap();
        assert!(!reply.exists());
        assert_eq!(
            h.app.pane_mailboxes[&child].back().unwrap().payload,
            "answer"
        );
        let (win, _) = h.app.find_pane_in_any_window(child).unwrap();
        h.app.windows[win]
            .panes
            .get_mut(&child)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .agent
            .as_mut()
            .unwrap()
            .state = AgentState::Idle;
        h.hidden_frame();
        assert_eq!(read(&reply)["status"], "idle");
    }

    #[test]
    fn agent_tool_wait_returns_question_then_validated_completion_while_hidden() {
        let mut h = HostHarness::new();
        let parent = h.add_focused_terminal();
        let parent_window_panes = h.app.windows[0].panes.len();
        let (child, _) = h.app.spawn_agent_worker_pane("tester", "pi", "sleep 30", "Verify",
            Some("agent-tools-test"), true, Some(parent), None,
            Some(json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}})), None, true, None).unwrap();
        assert_eq!(
            h.app.windows[0].panes.len(),
            parent_window_panes,
            "call must not split the parent's window"
        );
        assert!(!h
            .app
            .windows
            .iter()
            .flat_map(|w| w.panes.values())
            .any(|p| matches!(p, crate::host::pane::Pane::Portal(_))));
        h.app.agent_tool_children.insert(child, Some(parent));
        let root = h.workspace_root();
        let call_reply = root.join("call.json");
        h.app
            .park_agent_tool_wait(child, call_reply.to_str().unwrap());
        let done_reply = root.join("done.json");
        h.inject_ipc(crate::protocol::HostRequest::AgentDone {
            pane_id: child,
            result: Some("{\"wrong\":1}".into()),
            summary: None,
            response_file: done_reply.to_string_lossy().into_owned(),
        });
        h.hidden_frame();
        assert_eq!(read(&done_reply)["error_code"], "invalid_result");
        assert!(!call_reply.exists());
        h.inject_ipc(crate::protocol::HostRequest::AgentAsk {
            pane_id: child,
            question: "Which branch?".into(),
            response_file: root.join("ask.json").to_string_lossy().into_owned(),
        });
        h.hidden_frame();
        assert_eq!(read(&call_reply)["status"], "question");
        assert_eq!(read(&call_reply)["question"], "Which branch?");
        let completion = root.join("completion.json");
        h.app
            .park_agent_tool_wait(child, completion.to_str().unwrap());
        h.inject_ipc(crate::protocol::HostRequest::AgentDone {
            pane_id: child,
            result: Some("{\"ok\":true}".into()),
            summary: Some("Verified".into()),
            response_file: done_reply.to_string_lossy().into_owned(),
        });
        h.hidden_frame();
        let result = read(&completion);
        assert_eq!(result["status"], "completed");
        assert_eq!(result["summary"], "Verified");
        assert_eq!(result["result"], json!({"ok":true}));
        assert!(h.app.find_pane_in_any_window(child).is_none());
        assert_eq!(
            h.app.pane_results[&child].committed,
            Some(json!({"ok":true}))
        );
    }

    #[test]
    fn agent_tool_permissions_use_host_role_and_separate_handoff_allowlist() {
        let mut h = HostHarness::new();
        let parent = h.add_focused_terminal();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let dir = crate::agent::workspace_agents_dir(&root).join("lead");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENT.md"), "Lead").unwrap();
        std::fs::write(dir.join("settings.toml"), "[agent]\nid='lead'\ndisplay_name='Lead'\n[harness]\ncli='pi'\ncan_call=['tester']\ncan_handoff=['writer']").unwrap();
        // The pane runs under a lead grant: its delegation limits are fixed
        // in it, not read from the role file at call time (stint 1632).
        let mut grant = crate::broker::pane_authority::PaneAuthority::for_test(
            parent,
            crate::broker::pane_authority::Ceiling::Baseline,
            h.app.windows[0].context_id,
            root.clone(),
        );
        grant.role = Some("lead".into());
        grant.can_call = crate::broker::pane_authority::Delegation::only(&["tester".into()]);
        grant.can_handoff = crate::broker::pane_authority::Delegation::only(&["writer".into()]);
        let terminal = h.app.windows[0]
            .panes
            .get_mut(&parent)
            .unwrap()
            .as_terminal_mut()
            .unwrap();
        terminal.agent_role = Some("lead".into());
        terminal.replace_grant_for_test(grant);
        let ctx = h.app.windows[0].context_id;
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), ctx, "tester", false)
            .is_ok());
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), ctx, "writer", true)
            .is_ok());
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), ctx, "writer", false)
            .unwrap_err()
            .contains("can_call"));
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), ctx, "tester", true)
            .unwrap_err()
            .contains("can_handoff"));
        assert!(h
            .app
            .check_agent_tool_permission(Some(u64::MAX), ctx, "tester", false)
            .is_err());
    }
}

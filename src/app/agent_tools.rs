//! Agent tool calls use the worker lifecycle and the host's structured mailbox.
use super::PlexiApp;
use crate::protocol::AgentState;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(crate) struct AgentCaller<'a> {
    pub pane_id: Option<u64>,
    pub credential: Option<&'a str>,
    pub peer_ancestry: &'a [u32],
}

pub(crate) struct AgentCallRequest<'a> {
    pub role: &'a str,
    pub task: &'a str,
    pub parent_pane_id: Option<u64>,
    pub credential: Option<&'a str>,
    pub peer_ancestry: &'a [u32],
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

impl PlexiApp {
    fn agent_tool_caller(&self, caller: AgentCaller<'_>) -> Result<Option<u64>, String> {
        let peer = self
            .resolve_socket_peer_pane(caller.peer_ancestry)
            .map(|(id, _, _)| id);
        if peer.is_none() && caller.pane_id.is_none() && caller.credential.is_none() {
            // A terminal outside this host may start an unparented tool call.
            if caller.peer_ancestry.is_empty() {
                return Err("agent call requires an authenticated socket connection".into());
            }
            return Ok(None);
        }
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let verified = monitor
            .authenticate_call(caller.pane_id, caller.credential, peer)
            .map_err(|e| format!("permission_denied: caller identity: {e:?}"))?;
        if let Some(id) = verified.pane_id {
            if self.find_pane_in_any_window(id).is_none() {
                return Err(format!("caller pane {id} is no longer live"));
            }
        }
        Ok(verified.pane_id)
    }

    pub(crate) fn check_agent_tool_permission(
        &self,
        caller: Option<u64>,
        role: &str,
        handoff: bool,
    ) -> Result<std::path::PathBuf, String> {
        let win = match caller {
            Some(id) => self
                .find_pane_in_any_window(id)
                .map(|(w, _)| w)
                .ok_or_else(|| format!("caller pane {id} not found"))?,
            None => self.active_window,
        };
        let root = self
            .router
            .iter()
            .find(|c| c.context_id == self.windows[win].context_id)
            .map(|c| c.root.clone())
            .unwrap_or_else(|| self.windows[win].path.clone());
        let caller_role = caller
            .and_then(|id| self.windows[win].panes.get(&id))
            .and_then(|p| p.as_terminal())
            .and_then(|t| t.agent_role.as_deref());
        if let Some(caller_role) = caller_role {
            let workspace = crate::app::registry::resolve_agent_lookup_workspace_root(&root);
            let (def, _) = crate::cli::agent::resolve_agent_definition_in(
                caller_role,
                &crate::config::config_dir(),
                workspace.as_deref(),
            )?;
            let allowed = if handoff {
                &def.can_handoff
            } else {
                &def.can_call
            };
            if !allowed.iter().any(|r| r == role) {
                let field = if handoff { "can_handoff" } else { "can_call" };
                return Err(format!(
                    "permission_denied: agent '{caller_role}' does not list '{role}' in {field}"
                ));
            }
        }
        Ok(root)
    }

    pub(crate) fn start_agent_tool_call(
        &mut self,
        request: AgentCallRequest<'_>,
    ) -> Result<(), String> {
        if request.task.trim().is_empty() {
            return Err("task must be non-empty".into());
        }
        let parent = self.agent_tool_caller(AgentCaller {
            pane_id: request.parent_pane_id,
            credential: request.credential,
            peer_ancestry: request.peer_ancestry,
        })?;
        if request.handoff && parent.is_none() {
            return Err("handoff requires a live caller pane".into());
        }
        let root = self.check_agent_tool_permission(parent, request.role, request.handoff)?;
        let lookup_root = if parent.is_some() {
            root.clone()
        } else {
            request
                .cwd
                .map(std::path::PathBuf::from)
                .unwrap_or(root.clone())
        };
        let workspace = crate::app::registry::resolve_agent_lookup_workspace_root(&lookup_root);
        let (def, _) = crate::cli::agent::resolve_agent_definition_in(
            request.role,
            &crate::config::config_dir(),
            workspace.as_deref(),
        )?;
        let harness = def
            .harness
            .as_ref()
            .ok_or("agent role has no CLI harness")?;
        let adapter = crate::cli::agent::resolve_harness_adapter(harness)?;
        let handoff_result = if request.handoff {
            let old = parent.ok_or("handoff requires a caller")?;
            let raw = json!({"status":"handed_off","role":def.id,"task":request.task}).to_string();
            let mut result_state = self.pane_results.get(&old).cloned().unwrap_or_default();
            result_state
                .write_payload(&raw)
                .map_err(|error| format!("handoff completion: {}", error.message()))?;
            Some(raw)
        } else {
            None
        };
        let task = if request.handoff {
            let old = parent.ok_or("handoff requires a caller")?;
            let terminal = self
                .windows
                .iter()
                .find_map(|w| w.panes.get(&old))
                .and_then(|p| p.as_terminal());
            let packet = json!({"from_pane_id":old,"from_role":terminal.and_then(|t| t.agent_role.as_deref()),
                "session":terminal.and_then(|t| t.build_saved_agent_session()),"task":request.task});
            format!(
                "# Restore packet\n{packet}\n\n# Successor task\n{}",
                request.task
            )
        } else {
            request.task.to_string()
        };
        let command = crate::cli::agent::build_agent_spawn_command(&def, adapter, &task)?;
        let managed = request
            .worktree
            .map(|branch| crate::host::worktree::create_worktree(&lookup_root, branch))
            .transpose()?;
        let cwd = managed
            .as_ref()
            .map(|w| w.path.clone())
            .unwrap_or(lookup_root);
        let child_parent = if request.handoff {
            parent
                .and_then(|id| self.windows.iter().find_map(|w| w.panes.get(&id)))
                .and_then(|p| p.as_terminal())
                .and_then(|t| t.parent_pane_id)
        } else {
            parent
        };
        let subcontext = child_parent.map(|id| format!("agent-tools-{id}"));
        let origin_context = parent
            .and_then(|id| self.find_pane_in_any_window(id))
            .map(|(win, _)| self.windows[win].context_id);
        let spawned = self.spawn_agent_worker_pane(
            &def.id,
            adapter.harness.as_str(),
            &command,
            request.task,
            subcontext.as_deref(),
            harness.ephemeral,
            child_parent,
            Some(cwd),
            harness.result_schema.clone(),
            None,
            true,
            origin_context,
        );
        let (pane_id, _) = match spawned {
            Ok(pair) => pair,
            Err(error) => {
                if let Some(wt) = managed {
                    log::warn!(
                        "agent_call: launch failed; retaining worktree {}: {error}",
                        wt.path.display()
                    );
                }
                return Err(error);
            }
        };
        if let Some(wt) = managed {
            self.pane_worktrees.insert(pane_id, wt);
        }
        self.agent_tool_children.insert(pane_id, child_parent);
        log::info!("agent_call: spawned role={} parent={child_parent:?} child={pane_id} async={} handoff={}", def.id, request.asynchronous, request.handoff);
        if request.handoff {
            if let Some(old) = parent {
                // Preserve the successor reference as the caller's validated completion.
                let result = handoff_result.ok_or("missing prepared handoff completion")?;
                self.handle_pane_ipc_request(crate::protocol::AppRequest::AgentDone {
                    pane_id: old,
                    result: Some(result),
                    summary: Some(format!("Handed off to {}", def.id)),
                    response_file: request.response_file.to_string(),
                });
                // Handoff transfers ownership, independently of ordinary done's
                // optional ephemeral cleanup. The successor exists and the
                // prevalidated completion has been published before closing.
                if self.find_pane_in_any_window(old).is_some() {
                    self.close_pane_by_id(old);
                }
                log::info!("agent_handoff: closed caller={old} successor={pane_id}");
            }
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
        let parent = self.agent_tool_caller(caller)?;
        let (win, _) = self
            .find_pane_in_any_window(pane_id)
            .ok_or_else(|| format!("pane {pane_id} not found"))?;
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
                for (&id, pane) in &win.panes {
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
        self.conjoined_contexts
            .retain(|context, _| self.router.iter().any(|c| c.context_id == *context));
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
        let pane_id = pane_id.resolve().unwrap();
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
        let parent = h.add_test_pane();
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
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir());
        let token = monitor.issue_credential(
            Some(parent),
            h.app.windows[0].context_id,
            &root,
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
            crate::app::handle_socket_line(&wire, &h.ipc_tx, Some(&[]), reader.get_mut());
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
                h.inject_ipc(crate::protocol::AppRequest::AgentAsk {
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
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir());
        let token = monitor.issue_credential(
            Some(caller),
            h.app.windows[0].context_id,
            &root,
            "agent:lead",
        );
        let response = root.join("handoff.json");
        h.app
            .start_agent_tool_call(AgentCallRequest {
                role: "writer",
                task: "Continue",
                parent_pane_id: Some(caller),
                credential: Some(&token),
                peer_ancestry: &[],
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
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir());
        let token = monitor.issue_credential(
            Some(parent),
            h.app.windows[0].context_id,
            &root,
            "agent:lead",
        );
        let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        let response = root.join("handoff.json");
        let error = h
            .app
            .start_agent_tool_call(AgentCallRequest {
                role: "writer",
                task: "Continue",
                parent_pane_id: Some(parent),
                credential: Some(&token),
                peer_ancestry: &[],
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
        let parent = h.add_test_pane();
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
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir());
        let token = monitor.issue_credential(
            Some(parent),
            h.app.windows[0].context_id,
            &root,
            "agent:lead",
        );
        let reply = root.join("message.json");
        assert!(h
            .app
            .send_agent_tool_message(
                child,
                "answer",
                AgentCaller {
                    pane_id: Some(parent),
                    credential: None,
                    peer_ancestry: &[]
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
                    pane_id: Some(parent),
                    credential: Some(&token),
                    peer_ancestry: &[],
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
        let parent = h.add_test_pane();
        let (child, _) = h.app.spawn_agent_worker_pane("tester", "pi", "sleep 30", "Verify",
            Some("agent-tools-test"), true, Some(parent), None,
            Some(json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}})), None, true, None).unwrap();
        assert_eq!(
            h.app.windows[0].panes.len(),
            1,
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
        h.inject_ipc(crate::protocol::AppRequest::AgentDone {
            pane_id: child,
            result: Some("{\"wrong\":1}".into()),
            summary: None,
            response_file: done_reply.to_string_lossy().into_owned(),
        });
        h.hidden_frame();
        assert_eq!(read(&done_reply)["error_code"], "invalid_result");
        assert!(!call_reply.exists());
        h.inject_ipc(crate::protocol::AppRequest::AgentAsk {
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
        h.inject_ipc(crate::protocol::AppRequest::AgentDone {
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
        h.app.windows[0]
            .panes
            .get_mut(&parent)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .agent_role = Some("lead".into());
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), "tester", false)
            .is_ok());
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), "writer", true)
            .is_ok());
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), "writer", false)
            .unwrap_err()
            .contains("can_call"));
        assert!(h
            .app
            .check_agent_tool_permission(Some(parent), "tester", true)
            .unwrap_err()
            .contains("can_handoff"));
        assert!(h
            .app
            .check_agent_tool_permission(Some(u64::MAX), "tester", false)
            .is_err());
    }
}

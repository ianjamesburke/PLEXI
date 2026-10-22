//! Team orchestration over existing heads, worker panes, and validated results.
#[cfg(test)]
mod tests {
    use crate::testing::HostHarness;
    use serde_json::{json, Value};

    fn scripted_fixture() -> (HostHarness, std::path::PathBuf) {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let workspace = h.workspace_root();
        let dir = workspace
            .join(crate::config::workspace_channel_dir())
            .join("teams");
        std::fs::create_dir_all(&dir).unwrap();
        let mut team = crate::agent::team::builtin();
        for member in &mut team.members {
            member.harness = "script".into();
            member.command = Some("true".into());
        }
        std::fs::write(dir.join("test-team.toml"), toml::to_string(&team).unwrap()).unwrap();
        (h, workspace)
    }
    #[test]
    fn team_timeout_keeps_members_and_revokes_runs_with_failed_report() {
        let (mut h, workspace) = scripted_fixture();
        let reply = workspace.join("timeout.json");
        h.app.start_team(&json!({"workspace":workspace,"name":"test-team","question":"Choose","out":workspace.join("out")}), reply.to_str().unwrap()).unwrap();
        let run_ids = h.app.team_runs[0].run_ids.clone();
        let panes: Vec<_> = h.app.team_runs[0].members.iter().map(|(p, _)| *p).collect();
        // Exercise the same broker entry point used by a run-scoped MCP bearer.
        let denied = crate::agent::heads::handle_request(
            "call",
            &json!({"workspace":workspace,"run_id":run_ids[1],"tool":"agents.review","input_json":"{}"}),
        );
        assert_eq!(denied["error_code"], "permission_denied", "{denied}");
        h.app.team_runs[0].deadline = std::time::Instant::now();
        h.hidden_frame();
        let response: Value =
            serde_json::from_str(&std::fs::read_to_string(reply).unwrap()).unwrap();
        assert_eq!(response["ok"], false);
        assert!(workspace.join("out/RESULT.md").exists());
        assert!(h.app.team_runs.is_empty());
        for pane in panes {
            assert!(h.app.find_pane_in_any_window(pane).is_some());
        }
        for id in run_ids {
            let run = crate::agent::heads::handle_request(
                "show_run",
                &json!({"workspace":workspace,"id":id}),
            );
            assert_eq!(run["run"]["active"], false);
        }
    }
    #[test]
    fn team_failed_members_keep_ballot_and_evidence_below_quorum() {
        check_failed_team(None, 1, "FAILED");
    }
    #[test]
    fn team_quorum_publishes_partial_winner_and_excluded_member() {
        check_failed_team(Some(2), 2, "PARTIAL");
    }
    fn check_failed_team(quorum: Option<usize>, valid: usize, status: &str) {
        let (mut h, workspace) = scripted_fixture();
        if let Some(quorum) = quorum {
            let path = workspace
                .join(crate::config::workspace_channel_dir())
                .join("teams/test-team.toml");
            let raw = std::fs::read_to_string(&path).unwrap();
            std::fs::write(path, format!("quorum = {quorum}\n{raw}")).unwrap();
        }
        let reply = workspace.join("reply.json");
        h.app.start_team(&json!({"workspace":workspace,"name":"test-team","question":"Choose","out":workspace.join("out")}), reply.to_str().unwrap()).unwrap();
        let root = h.app.team_runs[0].root_pane;
        let panes: Vec<_> = h.app.team_runs[0].members.iter().map(|(p, _)| *p).collect();
        for (i, pane) in panes.iter().enumerate() {
            if i >= valid {
                h.app.park_agent_tool_wait(
                    *pane,
                    workspace.join(format!("wait-{i}.json")).to_str().unwrap(),
                );
            }
            let request = if i < valid {
                json!({"type":"agent_done", "pane_id":pane,
                    "result":json!({"ranking":["Proceed","Experiment","Defer"],"rationale":format!("Valid ballot {i}")}).to_string(),
                    "response_file":workspace.join(format!("done-{i}.json"))})
            } else {
                json!({"type":"set_agent_state","pane_id":pane,"state":"failed","agent":"script",
                    "event":"HarnessExit","detail":format!("harness exited with code {i} without agent done")})
            };
            h.inject_ipc(serde_json::from_value(request).unwrap());
        }
        h.hidden_frame();
        assert!(
            h.app.team_runs.is_empty(),
            "terminal failures must not wait for the deadline"
        );
        let response: Value =
            serde_json::from_str(&std::fs::read_to_string(reply).unwrap()).unwrap();
        assert_eq!(response["status"], status);
        let fleet = h.app.agents_view(crate::protocol::FleetFilter::All);
        for (i, pane) in panes.iter().enumerate().skip(valid) {
            let terminal = h
                .app
                .windows
                .iter()
                .find_map(|w| w.panes.get(pane))
                .unwrap();
            assert_eq!(
                terminal.agent().unwrap().state,
                crate::protocol::AgentState::Failed
            );
            let row = fleet
                .tasks
                .iter()
                .flat_map(|task| &task.rows)
                .find(|row| row.pane_id == *pane)
                .unwrap();
            assert_eq!(row.display_state, crate::agent::fleet::DisplayState::Failed);
            assert!(row.tail.as_ref().unwrap().contains("harness exited"));
            let wait: Value = serde_json::from_str(
                &std::fs::read_to_string(workspace.join(format!("wait-{i}.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(wait["status"], "failed");
        }
        let report = std::fs::read_to_string(workspace.join("out/RESULT.md")).unwrap();
        assert!(report.contains(status));
        assert!(report.contains("Valid ballot 0"));
        assert!(report.contains("failed: harness exited with code 2"));
        assert_eq!(report.contains("## Borda ranking"), quorum.is_some());
        if quorum.is_some() {
            assert!(report.contains("Excluded: strategist"));
        }
        for pane in panes.into_iter().chain([root]) {
            assert!(h.app.find_pane_in_any_window(pane).is_some());
        }
    }
    #[test]
    fn accepted_ballot_survives_late_harness_exit() {
        let (mut h, workspace) = scripted_fixture();
        let reply = workspace.join("reply.json");
        h.app.start_team(&json!({"workspace":workspace,"name":"test-team","question":"Choose","out":workspace.join("out")}), reply.to_str().unwrap()).unwrap();
        let pane = h.app.team_runs[0].members[0].0;
        h.inject_ipc(serde_json::from_value(json!({"type":"agent_done","pane_id":pane,
            "result":json!({"ranking":["Proceed","Experiment","Defer"],"rationale":"Accepted"}).to_string(),
            "response_file":workspace.join("done.json")})).unwrap());
        h.hidden_frame();
        h.inject_ipc(serde_json::from_value(json!({"type":"set_agent_state","pane_id":pane,"state":"failed","agent":"script","event":"HarnessExit","detail":"harness exited with code 0 without agent done"})).unwrap());
        h.hidden_frame();
        assert!(h.app.pane_results[&pane].committed.is_some());
        let terminal = h
            .app
            .windows
            .iter()
            .find_map(|w| w.panes.get(&pane))
            .unwrap()
            .as_terminal()
            .unwrap();
        assert!(terminal.agent_completed);
        assert_ne!(
            terminal.agent.as_ref().unwrap().state,
            crate::protocol::AgentState::Failed
        );
        h.app.team_runs[0].deadline = std::time::Instant::now();
        h.hidden_frame();
    }
    #[test]
    fn team_preserves_existing_result_and_refuses_caller_grant_widening() {
        let (mut h, workspace) = scripted_fixture();
        std::fs::write(workspace.join("RESULT.md"), "keep me").unwrap();
        let reply = workspace.join("reply.json");
        let request =
            json!({"workspace":workspace,"name":"test-team","question":"Choose","out":workspace});
        assert!(h
            .app
            .start_team(&request, reply.to_str().unwrap())
            .unwrap_err()
            .contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("RESULT.md")).unwrap(),
            "keep me"
        );
        let mut team = crate::agent::team::builtin();
        team.members[0].grants = vec!["agents.review=allow".into()];
        let path = workspace
            .join(crate::config::workspace_channel_dir())
            .join("teams/test-team.toml");
        std::fs::write(path, toml::to_string(&team).unwrap()).unwrap();
        let caller = h.add_test_pane();
        let request = json!({"workspace":workspace,"name":"test-team","question":"Choose","out":workspace.join("other"),"caller_pane":caller});
        assert!(h.app.start_team(&request, reply.to_str().unwrap()).is_err());
        assert!(h.app.team_runs.is_empty());
    }

    #[test]
    fn team_run_collects_three_agent_done_results_while_hidden_and_records_trace() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let workspace = h.workspace_root();
        let dir = workspace
            .join(crate::config::workspace_channel_dir())
            .join("teams");
        std::fs::create_dir_all(&dir).unwrap();
        let mut team = crate::agent::team::builtin();
        for member in &mut team.members {
            member.harness = "script".into();
            member.command = Some("true".into());
        }
        std::fs::write(dir.join("test-team.toml"), toml::to_string(&team).unwrap()).unwrap();
        let out = workspace.join("result");
        let reply = workspace.join("team-reply.json");
        h.inject_ipc(
            serde_json::from_value(json!({
                "type":"agents_api", "op":"team_run", "payload": {
                    "workspace": workspace, "name":"test-team", "question":"Which action?",
                    "out":out, "timeout":60
                }, "response_file":reply
            }))
            .unwrap(),
        );
        h.hidden_frame();
        assert!(
            !reply.exists(),
            "team run must park until all members finish"
        );
        let fleet = h.app.agents_view(crate::protocol::FleetFilter::All);
        let task = fleet
            .tasks
            .iter()
            .find(|t| t.rows.len() == 3)
            .expect("three members under one task");
        let members: Vec<_> = task.rows.iter().map(|r| r.pane_id).collect();
        let invalid_reply = workspace.join("invalid.json");
        h.inject_ipc(serde_json::from_value(json!({"type":"agent_done","pane_id":members[0],
            "result":json!({"ranking":["Proceed","Proceed","Defer"],"rationale":"duplicate"}).to_string(),
            "response_file":invalid_reply})).unwrap());
        h.hidden_frame();
        let invalid: Value =
            serde_json::from_str(&std::fs::read_to_string(invalid_reply).unwrap()).unwrap();
        assert_eq!(invalid["ok"], false);
        assert!(!reply.exists());
        for (i, pane) in members.iter().enumerate() {
            let mut ranking = team.options.clone();
            ranking.rotate_left(i);
            h.inject_ipc(serde_json::from_value(json!({
                "type":"agent_done", "pane_id":pane,
                "result":json!({"ranking":ranking,"rationale":format!("Reason {i}")}).to_string(),
                "response_file":workspace.join(format!("done-{i}.json"))
            })).unwrap());
            h.hidden_frame();
            if i < 2 {
                assert!(!reply.exists());
            }
        }
        h.hidden_frame();
        let response: Value =
            serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap();
        assert_eq!(response["ok"], true, "{response}");
        let result = std::fs::read_to_string(out.join("RESULT.md")).unwrap();
        for i in 0..3 {
            assert!(result.contains(&format!("Reason {i}")));
        }
        assert!(result.contains("Borda"));
        let (_, _, trace) =
            crate::cli::debug::build_show(&crate::config::config_dir(), Some("latest"), None, None)
                .unwrap();
        assert!(trace.iter().any(|e| e["kind"] == "team_start"));
        assert_eq!(
            trace
                .iter()
                .filter(|e| e["kind"] == "team_member_spawn")
                .count(),
            3
        );
        for pane in members {
            h.app.close_pane_by_id(pane);
        }
    }
}
use super::PlexiApp;
use crate::agent::team::{Ballot, MemberOutcome, Team};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) struct TeamRun {
    team: Team,
    question: String,
    workspace: PathBuf,
    out: PathBuf,
    response_file: String,
    root_pane: u64,
    context_id: u64,
    run_ids: Vec<String>,
    members: Vec<(u64, String)>,
    deadline: Instant,
}
fn api(workspace: &Path, op: &str, mut payload: Value) -> Result<Value, String> {
    payload["workspace"] = json!(workspace);
    let result = crate::agent::heads::handle_request(op, &payload);
    if result["ok"] == true {
        Ok(result)
    } else {
        Err(result["error"]
            .as_str()
            .unwrap_or("agents API failed")
            .to_string())
    }
}
fn field(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("missing {key}"))
}
impl PlexiApp {
    pub(crate) fn team_head_for_pane(&self, pane: u64) -> Option<String> {
        self.team_runs
            .iter()
            .flat_map(|run| &run.members)
            .find(|(id, _)| *id == pane)
            .map(|(_, head)| head.clone())
    }
    pub(crate) fn start_team(
        &mut self,
        payload: &Value,
        response_file: &str,
    ) -> Result<(), String> {
        let workspace = PathBuf::from(field(payload, "workspace")?)
            .canonicalize()
            .map_err(|e| format!("team workspace: {e}"))?;
        let name = field(payload, "name")?;
        let question = field(payload, "question")?;
        if question.trim().is_empty() {
            return Err("question cannot be empty".into());
        }
        let mut team = crate::agent::team::load(&workspace, &name)?;
        if let Some(options) = payload["options"].as_array().filter(|a| !a.is_empty()) {
            team.options = options
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "options must be strings".to_string())
                })
                .collect::<Result<_, _>>()?;
        }
        team.validate()?;
        let timeout = payload["timeout"].as_u64().unwrap_or(600);
        if !(1..=86400).contains(&timeout) {
            return Err("timeout must be 1..86400 seconds".into());
        }
        let caller = payload["caller_pane"].as_u64();
        if caller.is_some_and(|id| self.find_pane_in_any_window(id).is_none()) {
            return Err("caller pane not found".into());
        }
        let id = format!("team-{}", uuid::Uuid::new_v4().simple());
        let out = payload["out"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace.join("team-results").join(&id));
        if !out.is_absolute() {
            return Err("output directory must be absolute".into());
        }
        if out.join("RESULT.md").exists() {
            return Err("RESULT.md already exists; choose a new output directory".into());
        }
        std::fs::create_dir_all(&out).map_err(|e| format!("create {}: {e}", out.display()))?;
        let mut grants: Vec<String> = team.members.iter().flat_map(|m| m.grants.clone()).collect();
        // One parent policy per tool, using the strongest requested decision.
        grants.sort();
        grants.dedup();
        let mut union = std::collections::BTreeMap::<String, String>::new();
        for spec in grants {
            let (tool, decision) = spec
                .split_once('=')
                .ok_or_else(|| format!("invalid grant {spec}"))?;
            let rank = |d: &str| match d {
                "deny" => 0,
                "ask" => 1,
                "allow" | "allow:hard" | "allow:soft" => 2,
                _ => 3,
            };
            if union.get(tool).is_none_or(|old| rank(decision) > rank(old)) {
                union.insert(tool.into(), decision.into());
            }
        }
        let parent_grants: Vec<_> = union.iter().map(|(t, d)| format!("{t}={d}")).collect();
        let caller_human = caller.is_some_and(|p| self.is_human_owned_pane(p));
        let caller_ctx_root = caller.and_then(|p| self.pane_context_authority_root(p));
        let caller_ctx_denied = caller.is_some_and(|p| {
            let ws_canon = crate::platform::path::canonical_or_self(&workspace);
            caller_ctx_root.as_ref() != Some(&ws_canon)
                || !self.pane_may_access_context_root(p, &workspace)
        });
        api(
            &workspace,
            "create_head",
            json!({
                "name": id,
                "temporary": true,
                "grants": parent_grants,
                "caller_pane": caller,
                "caller_head": caller.and_then(|p| self.bound_head_for_pane(p)),
                "caller_human_pane": caller_human,
                "caller_context_root": caller_ctx_root,
                "caller_context_denied": caller_ctx_denied,
            }),
        )?;
        let parent = api(
            &workspace,
            "spawn_run",
            json!({"head":id,"journal_only":true,"admission":id}),
        )?;
        let parent_run = field(&parent["run"], "id")?;
        let spawned = self.spawn_agent_worker_pane(
            &name,
            "team",
            ":",
            &question,
            Some(&id),
            false,
            caller,
            Some(workspace.clone()),
            None,
            None,
            false,
            None,
        );
        let (root_pane, context_id) = match spawned {
            Ok(ids) => ids,
            Err(error) => {
                if let Err(cleanup) = api(&workspace, "finish_run", json!({"id":parent_run})) {
                    log::error!("team setup cleanup: {cleanup}");
                }
                return Err(error);
            }
        };
        // The coordinator is a new task root, even when an agent launched it.
        for win in &mut self.windows {
            if let Some(t) = win
                .panes
                .get_mut(&root_pane)
                .and_then(|p| p.as_terminal_mut())
            {
                t.parent_pane_id = None;
            }
        }
        let mut run = TeamRun {
            team,
            question,
            workspace,
            out,
            response_file: response_file.into(),
            root_pane,
            context_id,
            run_ids: vec![parent_run],
            members: vec![],
            deadline: Instant::now() + Duration::from_secs(timeout),
        };
        crate::host::flight_recorder::record(
            "team_start",
            Some(context_id),
            Some(root_pane),
            None,
            json!({"team":name,"question":run.question,"run_id":run.run_ids[0]}),
        );
        log::info!(
            "agent_team: start team={name} root_pane={root_pane} members={}",
            run.team.members.len()
        );
        if let Err(error) = self.spawn_team_members(&mut run) {
            // finish_team has already replied and retired the partial task.
            self.finish_team(&run, Some(&error));
            return Ok(());
        }
        self.team_runs.push(run);
        self.ctx.request_repaint();
        Ok(())
    }
    fn spawn_team_members(&mut self, run: &mut TeamRun) -> Result<(), String> {
        let cli = std::env::current_exe().map_err(|e| format!("team executable: {e}"))?;
        let quote = crate::host::shell::shell_quote;
        let completion = format!(
            "{} --socket {} agent done",
            quote(&cli.to_string_lossy()),
            quote(&crate::platform::ipc::endpoint())
        );
        for (i, member) in run.team.members.iter().enumerate() {
            let head = format!("{}-{i}", run.run_ids[0].replace('_', "-"));
            let child = api(
                &run.workspace,
                "delegate",
                json!({"parent_run":run.run_ids[0],"name":head,"grants":member.grants}),
            )?;
            let child_run = field(&child["run"], "id")?;
            run.run_ids.push(child_run.clone());
            let task = format!("Role: {}. Independently answer this question: {}\nRank these options, best first, exactly once each: {}\nReturn JSON {{\"ranking\":[...],\"rationale\":\"your reasoning\"}} using `{completion} --result '<JSON>'`. Do not consult other members. Your Plexi broker run id is {}. Stored grants: {}. Use the host MCP endpoint with PLEXI_HOST_MCP_PORT and PLEXI_HOST_MCP_TOKEN for brokered tools. The stored policy grants no OS sandbox privileges.", member.role, run.question, serde_json::to_string(&run.team.options).map_err(|e| e.to_string())?, child_run, member.grants.join(", "));
            let body = if let Some(script) = &member.command {
                format!("plexi_team_result=$(/bin/sh -c {}) && {} --result \"$plexi_team_result\" --summary {}", quote(script), completion, quote(&member.role))
            } else {
                let harness = crate::cli::AgentHarness::parse(&member.harness)
                    .ok_or_else(|| "invalid member harness".to_string())?;
                harness
                    .adapter()
                    .spawn_command(&task, member.model.as_deref(), None)
            };
            let command = format!(
                "export PLEXI_TEAM_RUN_ID={} PLEXI_TEAM_OPTIONS={}; {body}",
                quote(&child_run),
                quote(&serde_json::to_string(&run.team.options).map_err(|e| e.to_string())?)
            );
            let (pane, _) = self.spawn_agent_worker_pane(
                &member.role,
                &member.harness,
                &command,
                &task,
                None,
                false,
                Some(run.root_pane),
                Some(run.workspace.clone()),
                Some(run.team.schema()),
                Some(&field(&child, "run_token")?),
                false,
                None,
            )?;
            run.members.push((pane, head));
            crate::host::flight_recorder::record(
                "team_member_spawn",
                Some(run.context_id),
                Some(pane),
                None,
                json!({"task_pane":run.root_pane,"role":member.role,"harness":member.harness,"run_id":child_run,"grants":member.grants}),
            );
            log::info!(
                "agent_team: member_spawn task={} pane={pane} role={}",
                run.root_pane,
                member.role
            );
        }
        Ok(())
    }
    fn team_member_outcomes(&self, run: &TeamRun) -> Vec<Option<MemberOutcome>> {
        run.team
            .members
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let (pane, _) = run.members.get(i)?;
                if let Some(value) = self
                    .pane_results
                    .get(pane)
                    .and_then(|r| r.committed.clone())
                {
                    return Some(match serde_json::from_value::<Ballot>(value) {
                        Ok(ballot) => match ballot.validate(&run.team) {
                            Ok(()) => MemberOutcome::Ballot(ballot),
                            Err(error) => MemberOutcome::Failed(error),
                        },
                        Err(error) => MemberOutcome::Failed(format!("invalid ballot: {error}")),
                    });
                }
                let Some(terminal) = self
                    .windows
                    .iter()
                    .find_map(|w| w.panes.get(pane))
                    .and_then(|p| p.as_terminal())
                else {
                    return Some(MemberOutcome::Failed(
                        "pane closed before completion".into(),
                    ));
                };
                if let Some(agent) = terminal
                    .agent
                    .as_ref()
                    .filter(|a| a.state == crate::protocol::AgentState::Failed)
                {
                    return Some(MemberOutcome::Failed(
                        agent
                            .detail
                            .clone()
                            .unwrap_or_else(|| "harness failed".into()),
                    ));
                }
                if terminal.exited {
                    return Some(MemberOutcome::Failed(format!(
                        "harness exited {:?} without agent done",
                        terminal.exit_status
                    )));
                }
                None
            })
            .collect()
    }
    pub(crate) fn service_team_runs(&mut self, ctx: &egui::Context) {
        for run in std::mem::take(&mut self.team_runs) {
            let root_closed = self.find_pane_in_any_window(run.root_pane).is_none();
            if run.deadline <= Instant::now()
                || root_closed
                || self.team_member_outcomes(&run).iter().all(Option::is_some)
            {
                self.finish_team(
                    &run,
                    root_closed.then_some("team coordinator closed before completion"),
                );
            } else {
                self.team_runs.push(run);
            }
        }
        if let Some(deadline) = self.team_runs.iter().map(|r| r.deadline).min() {
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
    }
    fn finish_team(&mut self, run: &TeamRun, failure: Option<&str>) {
        let outcomes: Vec<_> = self
            .team_member_outcomes(run)
            .into_iter()
            .map(|outcome| {
                outcome.unwrap_or_else(|| {
                    failure
                        .map(|reason| MemberOutcome::Failed(reason.into()))
                        .unwrap_or(MemberOutcome::TimedOut)
                })
            })
            .collect();
        // Keep every pane and valid ballot. Missing/failed members remain inspectable.
        for ((pane, _), outcome) in run.members.iter().zip(&outcomes) {
            if !matches!(outcome, MemberOutcome::Ballot(_)) {
                self.mark_agent_failed(*pane, &outcome.summary());
            }
        }
        let report = crate::agent::team::report(&run.team, &run.question, &outcomes);
        let status = report.as_ref().map(|r| r.status).unwrap_or("FAILED");
        let report_text = report.as_ref().map(|r| r.text.clone()).unwrap_or_default();
        let mut outcome = report.and_then(|report| {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new_in(&run.out)
                .map_err(|e| format!("create result: {e}"))?;
            file.write_all(report.text.as_bytes())
                .map_err(|e| format!("write result: {e}"))?;
            file.persist_noclobber(run.out.join("RESULT.md"))
                .map_err(|e| format!("publish result: {e}"))?;
            Ok(())
        });
        for id in run.run_ids.iter().rev() {
            if let Err(error) = api(&run.workspace, "finish_run", json!({"id":id})) {
                log::error!("agent_team: finish run {id}: {error}");
                outcome = Err(error);
            }
        }
        let summaries: Vec<_> = run
            .team
            .members
            .iter()
            .zip(&outcomes)
            .map(|(member, outcome)| format!("{}: {}", member.role, outcome.summary()))
            .collect();
        let ok = outcome.is_ok() && status != "FAILED";
        let error = outcome
            .err()
            .or_else(|| (!ok).then(|| "team failed: quorum not met".into()));
        if ok {
            for win in &mut self.windows {
                if let Some(t) = win
                    .panes
                    .get_mut(&run.root_pane)
                    .and_then(|p| p.as_terminal_mut())
                {
                    t.agent_completed = true;
                    if let Some(agent) = &mut t.agent {
                        agent.state = crate::protocol::AgentState::Completed;
                        agent.detail = Some(format!("{status}: {}", summaries.join("; ")));
                    }
                }
            }
        } else {
            self.mark_agent_failed(
                run.root_pane,
                &format!(
                    "{}; {}",
                    error.as_deref().unwrap_or("team failed"),
                    summaries.join("; ")
                ),
            );
        }
        let reply = json!({"ok":ok,"status":if error.is_some() { "FAILED" } else { status },
            "error":error,"task_pane":run.root_pane,"members":run.members.iter().map(|(p,_)| p).collect::<Vec<_>>(),
            "member_summaries":summaries,"report":report_text,"result_path":run.out.join("RESULT.md")});
        log::info!(
            "agent_team: finished task={} status={} error={:?}",
            run.root_pane,
            reply["status"],
            error
        );
        crate::host::flight_recorder::record(
            "team_finish",
            Some(run.context_id),
            Some(run.root_pane),
            None,
            reply.clone(),
        );
        self.refresh_portal_context_states();
        self.mark_workspace_dirty();
        crate::rpc::write_json_response(&run.response_file, reply);
    }

    pub(crate) fn mark_agent_failed(&mut self, pane_id: u64, reason: &str) {
        let reason = reason.strip_prefix("failed: ").unwrap_or(reason);
        log::info!("agent_worker: failed pane={pane_id}: {reason}");
        let mut provenance = crate::host::pane_lifecycle::Provenance {
            source: crate::host::pane_lifecycle::Source::Hook,
            agent_label: "worker".into(),
            session_id: None,
            raw_event: Some("WorkerFailed".into()),
        };
        for win in &mut self.windows {
            if let Some(pane) = win.panes.get_mut(&pane_id) {
                let agent = pane.agent().cloned();
                if let Some(agent) = &agent {
                    provenance.agent_label = agent.agent.clone();
                    provenance.session_id = agent.session_id.clone();
                }
                if let Some(terminal) = pane.as_terminal_mut() {
                    terminal.agent_completed = false;
                    terminal.fleet_done_unseen = false;
                }
                let _ = pane.set_agent(Some(crate::protocol::PaneAgentState {
                    pane_id,
                    state: crate::protocol::AgentState::Failed,
                    agent: agent
                        .as_ref()
                        .map(|a| a.agent.clone())
                        .unwrap_or_else(|| "worker".into()),
                    detail: Some(reason.into()),
                    session_id: agent.and_then(|a| a.session_id),
                }));
                break;
            }
        }
        self.emit_pane_lifecycle(
            pane_id,
            crate::host::pane_lifecycle::PaneLifecycleEvent::TurnFailed { provenance },
        );
        self.finish_agent_tool_waits(
            pane_id,
            json!({"ok":false,"pane_id":pane_id,"status":"failed","error":reason}),
        );
        self.refresh_portal_context_states();
        self.mark_workspace_dirty();
    }
}

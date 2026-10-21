//! Team orchestration over existing heads, worker panes, and validated results.
#[cfg(test)]
mod tests {
    use crate::testing::HostHarness;
    use serde_json::{json, Value};

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
use crate::agent::team::{Ballot, Team};
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
        api(
            &workspace,
            "create_head",
            json!({"name":id,"temporary":true,"grants":parent_grants,
            "caller_pane":caller,"caller_head":caller.and_then(|p| self.bound_head_for_pane(p))}),
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
            self.finish_team(&run, Err(error.clone()));
            return Err(error);
        }
        self.team_runs.push(run);
        self.ctx.request_repaint();
        Ok(())
    }
    fn spawn_team_members(&mut self, run: &mut TeamRun) -> Result<(), String> {
        let cli = std::env::current_exe().map_err(|e| format!("team executable: {e}"))?;
        let quote = crate::host::shell::shell_quote;
        for (i, member) in run.team.members.iter().enumerate() {
            let head = format!("{}-{i}", run.run_ids[0].replace('_', "-"));
            let child = api(
                &run.workspace,
                "delegate",
                json!({"parent_run":run.run_ids[0],"name":head,"grants":member.grants}),
            )?;
            let child_run = field(&child["run"], "id")?;
            run.run_ids.push(child_run.clone());
            let task = format!("Role: {}. Independently answer this question: {}\nRank these options, best first, exactly once each: {}\nReturn JSON {{\"ranking\":[...],\"rationale\":\"your reasoning\"}} using `plexi agent done --result '<JSON>'`. Do not consult other members. Your Plexi broker run id is {}. Stored grants: {}. Use the host MCP endpoint with PLEXI_HOST_MCP_PORT and PLEXI_HOST_MCP_TOKEN for brokered tools. The stored policy grants no OS sandbox privileges.", member.role, run.question, serde_json::to_string(&run.team.options).map_err(|e| e.to_string())?, child_run, member.grants.join(", "));
            let body = if let Some(script) = &member.command {
                format!("plexi_team_result=$(/bin/sh -c {}) && {} agent done --result \"$plexi_team_result\" --summary {}", quote(script), quote(&cli.to_string_lossy()), quote(&member.role))
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
    pub(crate) fn service_team_runs(&mut self, ctx: &egui::Context) {
        for run in std::mem::take(&mut self.team_runs) {
            let values: Option<Vec<_>> = run
                .members
                .iter()
                .map(|(pane, _)| self.pane_results.get(pane)?.committed.clone())
                .collect();
            let outcome = if let Some(values) = values {
                Some(
                    values
                        .into_iter()
                        .map(serde_json::from_value::<Ballot>)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())
                        .and_then(|ballots| {
                            crate::agent::team::report(&run.team, &run.question, &ballots)
                        }),
                )
            } else if run.deadline <= Instant::now() {
                Some(Err(
                    "team deadline exceeded before every member submitted a valid ranking".into(),
                ))
            } else if run
                .members
                .iter()
                .any(|(id, _)| self.find_pane_in_any_window(*id).is_none())
                || self.find_pane_in_any_window(run.root_pane).is_none()
            {
                Some(Err("team pane closed before completion".into()))
            } else {
                None
            };
            if let Some(outcome) = outcome {
                self.finish_team(&run, outcome);
            } else {
                self.team_runs.push(run);
            }
        }
        if let Some(deadline) = self.team_runs.iter().map(|r| r.deadline).min() {
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
    }
    fn finish_team(&mut self, run: &TeamRun, outcome: Result<String, String>) {
        let outcome = outcome.and_then(|report| {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new_in(&run.out)
                .map_err(|e| format!("create result: {e}"))?;
            file.write_all(report.as_bytes())
                .map_err(|e| format!("write result: {e}"))?;
            file.persist_noclobber(run.out.join("RESULT.md"))
                .map_err(|e| format!("publish result: {e}"))?;
            Ok(())
        });
        let mut outcome = outcome;
        for id in run.run_ids.iter().rev() {
            if let Err(error) = api(&run.workspace, "finish_run", json!({"id":id})) {
                log::error!("agent_team: finish run {id}: {error}");
                outcome = Err(error);
            }
        }
        let reply = match outcome {
            Ok(()) => {
                for win in &mut self.windows {
                    if let Some(t) = win
                        .panes
                        .get_mut(&run.root_pane)
                        .and_then(|p| p.as_terminal_mut())
                    {
                        t.agent_completed = true;
                    }
                }
                json!({"ok":true,"task_pane":run.root_pane,"members":run.members.iter().map(|(p,_)| p).collect::<Vec<_>>(),"result_path":run.out.join("RESULT.md")})
            }
            Err(error) => {
                log::error!("agent_team: task={} failed: {error}", run.root_pane);
                for (pane, _) in &run.members {
                    self.close_pane_by_id(*pane);
                }
                self.close_pane_by_id(run.root_pane);
                json!({"ok":false,"error":error,"task_pane":run.root_pane})
            }
        };
        log::info!(
            "agent_team: finished task={} ok={}",
            run.root_pane,
            reply["ok"]
        );
        crate::host::flight_recorder::record(
            "team_finish",
            Some(run.context_id),
            Some(run.root_pane),
            None,
            reply.clone(),
        );
        crate::rpc::write_json_response(&run.response_file, reply);
    }
}

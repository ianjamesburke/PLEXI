//! Stint 1397: a named agent's memory folder and New chat. New chat archives
//! the chat `agent open` would return to and starts an empty one; the archive
//! still reopens from history, and every chat of the agent shares its memory.

use super::harness_tests::{
    pane_total, read_json_response, researcher_history_record, seed_pi_session, temp_response,
    terminal_role, AgentFocusFixture,
};
#[cfg(unix)]
use super::harness_tests::{assert_claude_memory_command, use_inert_agent_shell};
use super::*;
use crate::app::agent_open::AgentOpenOutcome;

fn terminal(h: &HostHarness, pane_id: PaneId) -> &crate::host::pane::TerminalPane {
    h.app
        .windows
        .iter()
        .find_map(|w| w.panes.get(&pane_id))
        .and_then(Pane::as_terminal)
        .expect("terminal pane")
}

fn terminal_mut(h: &mut HostHarness, pane_id: PaneId) -> &mut crate::host::pane::TerminalPane {
    h.app
        .windows
        .iter_mut()
        .find_map(|w| w.panes.get_mut(&pane_id))
        .and_then(Pane::as_terminal_mut)
        .expect("terminal pane")
}

fn pane_exists(h: &HostHarness, pane_id: PaneId) -> bool {
    h.app.windows.iter().any(|w| w.panes.contains_key(&pane_id))
}

/// Start `name` from the fixture profile as `agent start` does; returns its pane.
fn start_agent(h: &mut HostHarness, fx: &AgentFocusFixture, name: &str, caller: PaneId) -> PaneId {
    let payload = crate::cli::agent::resolve_agent_start_in_stopping_at(
        name,
        &fx.profile_dir,
        &fx.proj_dir,
        None,
        &fx.stops(),
    )
    .expect("resolve agent start payload");
    let req = fx.request(payload);
    let HostRequest::AgentSpawn { response_file, .. } = &req else {
        panic!("expected AgentSpawn");
    };
    let reply = response_file.clone();
    h.inject_from_pane(caller, req);
    h.run_frames(2);
    read_json_response(&reply)["pane_id"]
        .as_u64()
        .expect("started pane")
}

fn new_chat(
    h: &mut HostHarness,
    fx: &AgentFocusFixture,
    caller: PaneId,
) -> (PaneId, Option<String>) {
    let outcome = h
        .app
        .new_named_agent_chat(
            "researcher",
            &fx.proj_dir,
            Some(caller),
            &crate::broker::caller::RequestOrigin::HostUi,
        )
        .expect("new chat");
    h.run_frames(2);
    let AgentOpenOutcome::NewChat {
        pane_id,
        archived_run_id,
    } = outcome
    else {
        panic!("expected new_chat, got {outcome:?}");
    };
    (pane_id, archived_run_id)
}

fn memory_dir(name: &str) -> std::path::PathBuf {
    crate::config::shared_dir().join("agent_memory").join(name)
}

fn write_agent(fx: &AgentFocusFixture, id: &str, harness: &str) {
    let dir = fx.profile_dir.join("agents").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("AGENT.md"), format!("You are {id}.\n")).unwrap();
    std::fs::write(
        dir.join("settings.toml"),
        format!("[agent]\nid = \"{id}\"\ndisplay_name = \"{id}\"\n\n[harness]\ncli = \"{harness}\"\n"),
    )
    .unwrap();
}

#[test]
fn new_chat_archives_the_current_chat() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let caller = h.add_focused_terminal();
    let sessions = fx.home.path().join("pi-sessions");
    let _pi = seed_pi_session(&sessions, "sess-current");
    let current = start_agent(&mut h, &fx, "researcher", caller);
    terminal_mut(&mut h, current).agent_session_id = Some("sess-current".into());

    let (pane_id, archived) = new_chat(&mut h, &fx, caller);

    assert!(!pane_exists(&h, current), "the current chat's pane closed");
    assert_ne!(pane_id, current);
    assert_eq!(terminal_role(&h, pane_id).as_deref(), Some("researcher"));
    assert_eq!(
        terminal(&h, pane_id).agent_session_id,
        None,
        "the new chat is empty"
    );
    let run_id = archived.expect("the current chat was archived");
    let on_disk = crate::workspace::load_agent_history();
    let record = on_disk
        .iter()
        .find(|r| r.run_id == run_id)
        .expect("the archived chat is kept in history on disk");
    assert!(record.chat_archived);
    assert_eq!(record.session_id.as_deref(), Some("sess-current"));
    assert_eq!(record.pane_id, current);

    // `agent open` now returns to the new chat, never the archived one.
    let outcome = h
        .app
        .open_named_agent(
            "researcher",
            &fx.proj_dir,
            Some(caller),
            &crate::broker::caller::RequestOrigin::HostUi,
        )
        .expect("open");
    assert_eq!(outcome, AgentOpenOutcome::Focused { pane_id });
}

#[test]
fn new_chat_archives_the_last_closed_chat_when_no_pane_holds_it() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let caller = h.add_focused_terminal();
    let sessions = fx.home.path().join("pi-sessions");
    let _pi = seed_pi_session(&sessions, "sess-closed");
    let closed = researcher_history_record(&fx.proj_dir, "sess-closed", "pi");
    crate::workspace::append_agent_history(&closed).unwrap();
    h.app.fleet_history.borrow_mut().push(closed.clone());
    let before = pane_total(&h);

    let (pane_id, archived) = new_chat(&mut h, &fx, caller);

    assert_eq!(archived.as_deref(), Some(closed.run_id.as_str()));
    assert_eq!(pane_total(&h), before + 1, "only the new chat opened");
    assert_eq!(terminal(&h, pane_id).agent_session_id, None);
    assert!(crate::workspace::load_agent_history()
        .iter()
        .any(|r| r.run_id == closed.run_id && r.chat_archived));
}

#[test]
fn new_chat_with_no_chat_yet_only_starts_one() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let caller = h.add_focused_terminal();
    let (pane_id, archived) = new_chat(&mut h, &fx, caller);
    assert_eq!(archived, None);
    assert_eq!(terminal_role(&h, pane_id).as_deref(), Some("researcher"));
    let error = h
        .app
        .new_named_agent_chat(
            "no-such-agent",
            &fx.proj_dir,
            Some(caller),
            &crate::broker::caller::RequestOrigin::HostUi,
        )
        .expect_err("unknown agents fail before anything is archived");
    assert!(error.contains("no-such-agent"), "{error}");
}

#[test]
fn archived_chat_reopens_from_history() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let caller = h.add_focused_terminal();
    let sessions = fx.home.path().join("pi-sessions");
    let _pi = seed_pi_session(&sessions, "sess-archived");
    let current = start_agent(&mut h, &fx, "researcher", caller);
    terminal_mut(&mut h, current).agent_session_id = Some("sess-archived".into());
    let (_, archived) = new_chat(&mut h, &fx, caller);
    let run_id = archived.expect("archived");

    let reply = temp_response(fx.home.path(), "agent-reopen");
    h.inject_in_process(HostRequest::AgentReopen {
        query: run_id.clone(),
        subcontext: None,
        response_file: reply.clone(),
    });
    h.run_frames(2);

    let reply = read_json_response(&reply);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["run_id"], run_id.as_str());
    let reopened = reply["pane_id"].as_u64().expect("reopened pane");
    assert_eq!(
        terminal(&h, reopened).agent_session_id.as_deref(),
        Some("sess-archived")
    );
    assert!(terminal(&h, reopened)
        .last_resumed_command
        .as_deref()
        .is_some_and(|c| c.contains("sess-archived")));
}

#[test]
fn new_chat_preserves_named_agent_memory_files() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let caller = h.add_focused_terminal();
    let memory = memory_dir("researcher");
    let resolve_start = || {
        crate::cli::agent::resolve_agent_start_in_stopping_at(
            "researcher",
            &fx.profile_dir,
            &fx.proj_dir,
            None,
            &fx.stops(),
        )
        .unwrap()
    };
    let payload = resolve_start();
    let command = payload["command"].as_str().unwrap();
    assert!(command.contains(&*memory.to_string_lossy()), "{command}");

    let first = start_agent(&mut h, &fx, "researcher", caller);
    assert!(
        memory.is_dir(),
        "created when the first named chat launches"
    );
    std::fs::write(memory.join("facts.md"), "the build is just pr-candidate\n").unwrap();
    terminal_mut(&mut h, first).agent_session_id = Some("sess-1".into());

    let (second, _) = new_chat(&mut h, &fx, caller);
    assert!(pane_exists(&h, second));
    assert_eq!(
        std::fs::read_to_string(memory.join("facts.md")).unwrap(),
        "the build is just pr-candidate\n",
        "New chat preserves the existing memory file"
    );
    assert_eq!(resolve_start()["command"], payload["command"]);
    // The definition folder is never rewritten.
    let def_dir = fx.profile_dir.join("agents").join("researcher");
    assert_eq!(
        std::fs::read_to_string(def_dir.join("AGENT.md")).unwrap(),
        "You are a researcher.\n"
    );

    let ephemeral = crate::cli::agent::resolve_agent_spawn_in_stopping_at(
        "researcher",
        &fx.profile_dir,
        &fx.proj_dir,
        Some("task"),
        None,
        Some(true),
        &fx.stops(),
    )
    .unwrap();
    assert!(!ephemeral["command"]
        .as_str()
        .unwrap()
        .contains("memory folder"));
}

#[cfg(unix)]
#[test]
fn archived_claude_chat_reopens_with_its_agent_memory() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    write_agent(&fx, "researcher", "claude");
    let _shell = use_inert_agent_shell(fx.home.path());
    h.add_focused_terminal();
    let mut record = researcher_history_record(&fx.proj_dir, "archived-claude", "claude");
    record.chat_archived = true;
    let memory = memory_dir("researcher");
    let canonical_memory = crate::config::shared_dir()
        .canonicalize()
        .unwrap()
        .join("agent_memory/researcher");
    for (ephemeral, has_binding) in [(false, false), (true, true), (false, true)] {
        record.ephemeral = ephemeral;
        record.claude_memory_dir = has_binding.then(|| canonical_memory.clone());
        let reply = h.app.reopen_agent_record(
            &record,
            None,
            None,
            crate::broker::pane_authority::GrantSource::OutsideProcess,
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["session_id"], "archived-claude");
        let command = reply["command"].as_str().unwrap();
        if ephemeral || !has_binding {
            assert_eq!(command, "claude --resume archived-claude");
            assert!(!memory.exists());
        } else {
            assert_claude_memory_command(command, &memory, " --resume archived-claude");
            assert!(memory.is_dir());
        }
        let pane = terminal(&h, reply["pane_id"].as_u64().unwrap());
        assert_eq!(pane.last_resumed_command.as_deref(), Some(command));
    }
}

#[cfg(unix)]
#[test]
fn claude_standby_wake_binds_named_memory_for_resume_and_fallback() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    write_agent(&fx, "researcher", "claude");
    let _shell = use_inert_agent_shell(fx.home.path());
    let sessions = fx.home.path().join("claude-projects");
    std::fs::create_dir_all(sessions.join("-proj")).unwrap();
    std::fs::write(sessions.join("-proj").join("saved-claude.jsonl"), "{}\n").unwrap();
    let _sessions = crate::cli::agent::set_test_claude_projects_dir(sessions);
    let memory = memory_dir("researcher");
    let caller = h.add_focused_terminal();

    for (session_id, cached_command, args) in [
        (
            "saved-claude",
            Some("claude --resume saved-claude"),
            " --resume saved-claude",
        ),
        ("missing-claude", None, ""),
    ] {
        let pane_id = start_agent(&mut h, &fx, "researcher", caller);
        let pane = terminal_mut(&mut h, pane_id);
        pane.agent_session_id = Some(session_id.into());
        pane.standby_session = pane.build_saved_agent_session();
        pane.standby_cwd = Some(fx.proj_dir.clone());
        pane.standby_resume_command = cached_command.map(str::to_owned);

        let command = pane.resume_from_standby().unwrap();
        assert_claude_memory_command(&command, &memory, args);
        assert_eq!(pane.last_resumed_command.as_deref(), Some(command.as_str()));
        assert_eq!(
            pane.agent_session_id.as_deref(),
            cached_command.map(|_| session_id)
        );
    }
}

#[cfg(unix)]
#[test]
fn claude_reopen_keeps_the_launched_memory_after_workspace_shadowing() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    write_agent(&fx, "researcher", "claude");
    let _shell = use_inert_agent_shell(fx.home.path());
    let caller = h.add_focused_terminal();
    let pane_id = start_agent(&mut h, &fx, "researcher", caller);
    let pane = terminal_mut(&mut h, pane_id);
    pane.agent_session_id = Some("saved-claude".into());
    let saved = pane.build_saved_agent_session().unwrap();
    let saved = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    let record = pane
        .build_history_record(1, "test", fx.proj_dir.clone())
        .unwrap();
    let record = serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();

    let shadow = crate::agent::workspace_agents_dir(&fx.proj_dir).join("researcher");
    std::fs::create_dir_all(&shadow).unwrap();
    for file in ["AGENT.md", "settings.toml"] {
        std::fs::copy(
            fx.profile_dir.join("agents/researcher").join(file),
            shadow.join(file),
        )
        .unwrap();
    }

    let reply = h.app.reopen_agent_record(
        &record,
        None,
        None,
        crate::broker::pane_authority::GrantSource::OutsideProcess,
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_claude_memory_command(
        reply["command"].as_str().unwrap(),
        &memory_dir("researcher"),
        " --resume saved-claude",
    );

    let pane = terminal_mut(&mut h, pane_id);
    pane.standby_session = Some(saved);
    pane.standby_cwd = Some(fx.proj_dir.clone());
    pane.standby_resume_command = Some("claude --resume saved-claude".into());
    let command = pane.resume_from_standby().unwrap();
    assert_claude_memory_command(
        &command,
        &memory_dir("researcher"),
        " --resume saved-claude",
    );
    assert!(!fx.proj_dir.join(".plexi/agent_memory/researcher").exists());
}

#[test]
fn two_named_agents_never_share_memory_or_chats() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    write_agent(&fx, "writer", "pi");
    let caller = h.add_focused_terminal();
    let researcher = start_agent(&mut h, &fx, "researcher", caller);
    let writer = start_agent(&mut h, &fx, "writer", caller);
    assert_eq!(terminal_role(&h, writer).as_deref(), Some("writer"));
    let (researcher_memory, writer_memory) = (memory_dir("researcher"), memory_dir("writer"));
    assert_ne!(researcher_memory, writer_memory);
    std::fs::write(researcher_memory.join("r.md"), "researcher\n").unwrap();
    std::fs::write(writer_memory.join("w.md"), "writer\n").unwrap();

    let (_, archived) = new_chat(&mut h, &fx, caller);

    assert!(archived.is_some());
    assert!(!pane_exists(&h, researcher));
    assert!(
        pane_exists(&h, writer),
        "the other agent's chat is untouched"
    );
    assert!(
        !h.app
            .fleet_history
            .borrow()
            .iter()
            .any(|r| r.config_name.as_deref() == Some("writer")),
        "nothing of the other agent was archived"
    );
    assert_eq!(
        std::fs::read_to_string(writer_memory.join("w.md")).unwrap(),
        "writer\n"
    );
    assert_eq!(
        std::fs::read_to_string(researcher_memory.join("r.md")).unwrap(),
        "researcher\n"
    );
    assert!(!writer_memory.join("r.md").exists() && !researcher_memory.join("w.md").exists());
}

/// After New chat and a host restart, `agent open` returns the new chat: the
/// restored pane when it is still open, else its closed record, and never the
/// archived chat, even when the new chat has nothing resumable.
#[test]
fn agent_open_after_restart_returns_the_new_chat_never_the_archived_one() {
    for new_chat_state in ["open", "closed_with_session", "closed_empty"] {
        let mut h = HostHarness::new();
        let fx = AgentFocusFixture::new();
        let caller = h.add_focused_terminal();
        let sessions = fx.home.path().join("pi-sessions");
        let _old = seed_pi_session(&sessions, "sess-old");
        let _new = seed_pi_session(&sessions, "sess-new");
        let old = start_agent(&mut h, &fx, "researcher", caller);
        terminal_mut(&mut h, old).agent_session_id = Some("sess-old".into());
        let (fresh, archived) = new_chat(&mut h, &fx, caller);
        assert!(archived.is_some(), "{new_chat_state}");
        match new_chat_state {
            "open" => {}
            "closed_with_session" => {
                terminal_mut(&mut h, fresh).agent_session_id = Some("sess-new".into());
                h.app.close_pane_by_id(fresh);
            }
            _ => h.app.close_pane_by_id(fresh),
        }
        assert!(
            h.app.save_workspace_now(),
            "{new_chat_state}: workspace saved"
        );
        drop(h);

        let log = crate::testing::test_log::capture();
        let mut restarted = HostHarness::from_profile_dir(&fx.profile_dir);
        let caller = restarted.add_focused_terminal();
        // A closed chat records its pane's real directory, which the OS
        // reports resolved (`/private/var` for a `/var` temp dir).
        let cwd = fx.proj_dir.canonicalize().unwrap();
        let outcome = restarted
            .app
            .open_named_agent("researcher", &cwd, Some(caller), &crate::broker::caller::RequestOrigin::HostUi)
            .expect("open after restart");
        match (new_chat_state, &outcome) {
            ("open", AgentOpenOutcome::Focused { pane_id }) => {
                assert_eq!(*pane_id, fresh, "the restored new chat is focused");
            }
            ("closed_with_session", AgentOpenOutcome::Resumed { session_id, .. }) => {
                assert_eq!(session_id, "sess-new");
            }
            ("closed_empty", AgentOpenOutcome::Started { .. }) => {
                let skipped = log.lines().into_iter().any(|line| {
                    line.contains("not resuming") && line.contains("a New chat archived it")
                });
                assert!(
                    skipped,
                    "the archived chat was found and skipped, not missed"
                );
            }
            _ => panic!("{new_chat_state}: unexpected {outcome:?}"),
        }
    }
}

/// Codex 3290 r3 B4: a new chat asked for from inside the chat it replaces.
/// The archive closes the caller's own pane; the replacement is launched
/// for the caller as classified again after that close (as alpha resolved
/// live ancestry), never with the retired pane as its parent.
#[test]
fn new_chat_from_the_pane_it_replaces_names_no_retired_parent() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let launcher = h.add_focused_terminal();
    let _other = h.add_focused_terminal();
    let sessions = fx.home.path().join("pi-sessions");
    let _pi = seed_pi_session(&sessions, "sess-self");
    let current = start_agent(&mut h, &fx, "researcher", launcher);
    terminal_mut(&mut h, current).agent_session_id = Some("sess-self".into());
    let witness = crate::broker::caller::PeerWitness::for_live_pids(&[h.shell_pid(current)]);
    let origin = crate::broker::caller::RequestOrigin::Socket(
        h.app.classify_socket_peer(Some(&witness)),
    );
    assert!(matches!(
        origin,
        crate::broker::caller::RequestOrigin::Socket(crate::broker::caller::SocketCaller::Pane(ref p))
            if p.terminal_pane == current
    ));
    let outcome = h
        .app
        .new_named_agent_chat("researcher", &fx.proj_dir, None, &origin)
        .expect("new chat");
    h.run_frames(2);
    let AgentOpenOutcome::NewChat { pane_id, .. } = outcome else {
        panic!("expected new_chat, got {outcome:?}");
    };
    assert!(!pane_exists(&h, current), "the caller's chat was archived");
    let replacement = terminal(&h, pane_id);
    assert_ne!(replacement.parent_pane_id, Some(current), "a retired pane is the parent");
    assert!(
        h.app.pane_authority(pane_id).is_some_and(|a| a.grant().sealed_reason().is_none()),
        "the replacement launched with a usable grant"
    );
}

/// Codex 3290 r4 S2: a new chat asked for from inside the chat it replaces,
/// by a caller whose process walk could not be completed past that chat's
/// shell. Without the chat's pane the caller cannot be placed, so the
/// request is refused before anything closes: the current chat survives.
#[test]
fn new_chat_that_cannot_launch_refuses_before_closing_the_chat() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let launcher = h.add_focused_terminal();
    let sessions = fx.home.path().join("pi-sessions");
    let _pi = seed_pi_session(&sessions, "sess-keep");
    let current = start_agent(&mut h, &fx, "researcher", launcher);
    terminal_mut(&mut h, current).agent_session_id = Some("sess-keep".into());
    let mut witness = crate::broker::caller::PeerWitness::for_live_pids(&[h.shell_pid(current)]);
    witness.end = crate::broker::caller::AncestryEnd::LookupFailed;
    let origin = crate::broker::caller::RequestOrigin::Socket(
        h.app.classify_socket_peer(Some(&witness)),
    );
    assert!(matches!(
        origin,
        crate::broker::caller::RequestOrigin::Socket(crate::broker::caller::SocketCaller::Pane(_))
    ));
    let refused = h.app.new_named_agent_chat("researcher", &fx.proj_dir, None, &origin);
    assert!(refused.is_err(), "{refused:?}");
    assert!(pane_exists(&h, current), "the current chat was closed for a launch that could not happen");
    assert!(
        !crate::workspace::load_agent_history()
            .iter()
            .any(|record| record.pane_id == current && record.chat_archived),
        "the current chat was archived"
    );
}

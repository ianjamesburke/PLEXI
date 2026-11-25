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
    let payload = crate::cli::agent::agent_start_payload(name, &fx.proj_dir, None, &fx.stops())
        .expect("resolve agent start payload");
    #[cfg(unix)]
    let _shell = use_inert_agent_shell(fx.home.path());
    let mut payload = payload;
    payload["response_file"] = serde_json::json!(temp_response(fx.home.path(), "agent-start"));
    let req = serde_json::from_value(payload).unwrap();
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
    // The definition folder is never rewritten.
    let def_dir = fx.profile_dir.join("agents").join("researcher");
    assert_eq!(
        std::fs::read_to_string(def_dir.join("AGENT.md")).unwrap(),
        "You are a researcher.\n"
    );

    let ephemeral = crate::pane_ops::create::ConfiguredAgentLaunch::resolve(
        "researcher",
        &fx.proj_dir,
        Some("task"),
        Some(true),
    )
    .unwrap();
    assert!(!ephemeral.command.contains("memory folder"));
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
    for (ephemeral, has_binding) in [(false, false), (true, false), (false, true)] {
        record.ephemeral = ephemeral;
        record.claude_memory = has_binding.then(|| {
            serde_json::from_value(serde_json::json!({"scope": "global", "agent_id": "researcher"}))
                .unwrap()
        });
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

        let command = pane.resume_from_standby().unwrap().unwrap();
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
    let command = pane.resume_from_standby().unwrap().unwrap();
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

#[test]
fn admission_rejects_agent_id_aliases_before_lookup() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let caller = h.add_focused_terminal();
    let origin = crate::broker::caller::RequestOrigin::HostUi;
    for alias in ["researcher/", "./researcher", " researcher"] {
        let err = h
            .app
            .open_named_agent(alias, &fx.proj_dir, Some(caller), &origin)
            .expect_err("aliases must not resolve");
        assert!(
            err.contains("invalid agent") && err.contains(alias),
            "{err}"
        );
    }
    #[cfg(unix)]
    let _shell = use_inert_agent_shell(fx.home.path());
    assert!(
        h.app
            .open_named_agent("default", &fx.proj_dir, Some(caller), &origin)
            .is_ok()
    );
}

#[cfg(unix)]
fn capture_claude_argv(dir: &std::path::Path) -> crate::host::shell::TestShellOverrideGuard {
    use std::os::unix::fs::PermissionsExt as _;
    let shell = dir.join("capture-shell");
    let claude = dir.join("claude");
    std::fs::write(
        &shell,
        format!(
            "#!/bin/sh\nexport PATH='{dir}':/usr/bin:/bin\nunset ENV\n\
             if [ \"$2\" = -c ]; then exec /bin/sh -c \"$3\"; fi\n\
             /bin/stty -icanon -echo min 1 time 0 || exit 1\n\
             if [ \"$1\" = -l ]; then touch '{dir}/ready-'\"$PLEXI_PANE_ID\"; fi\n\
             while IFS= read -r line; do /bin/sh -c \"$line\"; done\n",
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\nprintf '%s\\0' \"$@\" >> '{}/argv-'\"$PLEXI_PANE_ID\"\n",
            dir.display()
        ),
    )
    .unwrap();
    for file in [&shell, &claude] {
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    crate::host::shell::set_test_shell_override(shell.to_string_lossy())
}

#[cfg(unix)]
fn captured_argv(dir: &std::path::Path, pane: PaneId) -> Vec<String> {
    let path = dir.join(format!("argv-{pane}"));
    for _ in 0..100 {
        if let Ok(args) = std::fs::read_to_string(&path) {
            if let Some(args) = args.strip_suffix('\0') {
                std::fs::remove_file(&path).unwrap();
                return args.split('\0').map(str::to_owned).collect();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("no Claude argv captured at {}", path.display());
}

#[cfg(unix)]
fn bound_path(argv: &[String]) -> std::path::PathBuf {
    let settings: Vec<_> = argv.windows(2).filter(|p| p[0] == "--settings").collect();
    assert_eq!(settings.len(), 1, "{argv:?}");
    let settings: serde_json::Value = serde_json::from_str(&settings[0][1]).unwrap();
    assert!(settings["hooks"]["SessionStart"].is_array());
    assert!(settings.get("autoMemoryEnabled").is_none());
    std::path::PathBuf::from(settings["autoMemoryDirectory"].as_str().unwrap())
}

#[cfg(unix)]
fn cli_launch(
    h: &mut HostHarness,
    fx: &AgentFocusFixture,
    caller: PaneId,
    mut payload: serde_json::Value,
) -> PaneId {
    let response = temp_response(fx.home.path(), "memory-admission");
    payload["response_file"] = serde_json::json!(response);
    h.inject_from_pane(caller, serde_json::from_value(payload).unwrap());
    h.run_frames(2);
    let reply = read_json_response(&response);
    assert_eq!(reply["ok"], true, "{reply}");
    reply["pane_id"].as_u64().unwrap()
}

#[cfg(unix)]
fn shadow_researcher(fx: &AgentFocusFixture, workspace: &std::path::Path, prompt: &str) {
    let dir = crate::agent::workspace_agents_dir(workspace).join("researcher");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        fx.profile_dir.join("agents/researcher/settings.toml"),
        dir.join("settings.toml"),
    )
    .unwrap();
    std::fs::write(dir.join("AGENT.md"), prompt).unwrap();
}

#[cfg(unix)]
#[test]
fn completion_round_definition_lookup_uses_canonical_workspace() {
    let fx = AgentFocusFixture::new();
    let agent_dir = crate::agent::workspace_agents_dir(&fx.proj_dir).join("researcher");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // A missing persona reports the actual path used for definition lookup.
    let alias = fx.home.path().join("project-alias");
    std::os::unix::fs::symlink(&fx.proj_dir, &alias).unwrap();
    let error = crate::pane_ops::create::ConfiguredAgentLaunch::resolve_in(
        "researcher",
        &alias,
        None,
        None,
        &fx.profile_dir,
        &fx.stops(),
    )
    .unwrap_err();
    let expected = agent_dir.canonicalize().unwrap();
    assert!(error.contains(expected.to_str().unwrap()), "{error}");
}

#[cfg(unix)]
#[test]
fn admission_builds_claude_argv_and_identity_from_the_same_definition() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let _shell = capture_claude_argv(fx.home.path());
    let caller = h.add_focused_terminal();
    write_agent(&fx, "researcher", "claude");
    let payload = crate::cli::agent::agent_start_payload(
        "researcher",
        &fx.proj_dir,
        Some("Do the task"),
        &fx.stops(),
    )
    .unwrap();
    shadow_researcher(&fx, &fx.proj_dir, "Workspace persona at admission");
    let repo_notes = fx.proj_dir.join(".plexi/agent_memory/researcher");
    std::fs::create_dir_all(&repo_notes).unwrap();
    std::fs::write(repo_notes.join("MEMORY.md"), "Repository-controlled notes").unwrap();
    let second = fx.home.path().join("second-project");
    shadow_researcher(&fx, &second, "Second workspace persona");

    for (cwd, prompt) in [
        (&fx.proj_dir, "Workspace persona at admission"),
        (&second, "Second workspace persona"),
    ] {
        let mut request = payload.clone();
        request["cwd"] = serde_json::json!(cwd);
        let pane_id = cli_launch(&mut h, &fx, caller, request);
        let argv = captured_argv(fx.home.path(), pane_id);
        assert!(argv.iter().any(|arg| arg.contains(prompt)), "{argv:?}");
        let root = cwd.canonicalize().unwrap();
        assert_eq!(
            bound_path(&argv),
            root.join(".plexi/agent_memory/researcher")
        );
        let saved =
            serde_json::to_value(terminal(&h, pane_id).build_saved_agent_session().unwrap())
                .unwrap();
        assert_eq!(
            saved["claude_memory"],
            serde_json::json!({"scope": {"workspace_root": root}, "agent_id": "researcher"})
        );
        assert!(saved.get("claude_memory_dir").is_none());

        let history = terminal(&h, pane_id)
            .build_history_record(1, "test", cwd.to_path_buf())
            .unwrap();
        let history = serde_json::from_value(serde_json::to_value(history).unwrap()).unwrap();
        std::fs::remove_dir_all(crate::agent::workspace_agents_dir(cwd).join("researcher"))
            .unwrap();
        let reply = h.app.reopen_agent_record(
            &history,
            None,
            None,
            crate::broker::pane_authority::GrantSource::OutsideProcess,
        );
        assert_eq!(reply["ok"], true, "{reply}");
        let reopened = reply["pane_id"].as_u64().unwrap();
        assert_eq!(
            bound_path(&captured_argv(fx.home.path(), reopened)),
            root.join(".plexi/agent_memory/researcher")
        );
    }
    assert_eq!(
        std::fs::read_to_string(repo_notes.join("MEMORY.md")).unwrap(),
        "Repository-controlled notes"
    );
}

#[cfg(unix)]
#[test]
fn admission_raw_claude_command_cannot_attest_named_memory() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let _shell = capture_claude_argv(fx.home.path());
    let caller = h.add_focused_terminal();
    write_agent(&fx, "researcher", "claude");
    let pane_id = cli_launch(
        &mut h,
        &fx,
        caller,
        serde_json::json!({
            "type": "agent_spawn", "role": "researcher", "harness": "claude",
            "command": "claude --resume raw-claude", "cwd": fx.proj_dir,
        }),
    );
    assert_eq!(
        captured_argv(fx.home.path(), pane_id),
        ["--resume", "raw-claude"]
    );
    let saved =
        serde_json::to_value(terminal(&h, pane_id).build_saved_agent_session().unwrap()).unwrap();
    assert!(
        saved.get("claude_memory").is_none() && saved.get("claude_memory_dir").is_none(),
        "{saved}"
    );
    assert!(!memory_dir("researcher").exists());
}

#[cfg(unix)]
#[test]
fn admission_legacy_claude_paths_are_ignored_on_reopen_and_restart() {
    let mut h = HostHarness::new();
    let fx = AgentFocusFixture::new();
    let _shell = capture_claude_argv(fx.home.path());
    let sessions = fx.home.path().join("claude-projects");
    std::fs::create_dir_all(sessions.join("-proj")).unwrap();
    std::fs::write(sessions.join("-proj/legacy-claude.jsonl"), "{}\n").unwrap();
    let _sessions = crate::cli::agent::set_test_claude_projects_dir(sessions);
    let caller = h.add_focused_terminal();
    write_agent(&fx, "researcher", "claude");
    let payload = crate::cli::agent::agent_spawn_payload(
        "researcher",
        &fx.proj_dir,
        Some("ephemeral task"),
        None,
        Some(true),
        &fx.stops(),
    )
    .unwrap();
    let ephemeral = cli_launch(&mut h, &fx, caller, payload);
    let argv = captured_argv(fx.home.path(), ephemeral);
    assert!(
        argv.iter().all(|arg| !arg.contains("autoMemoryDirectory")),
        "{argv:?}"
    );
    let pane = terminal_mut(&mut h, ephemeral);
    assert!(pane.ephemeral);
    pane.agent_session_id = Some("legacy-claude".into());
    let expired = serde_json::to_value(
        pane.build_history_record(1, "test", fx.proj_dir.clone())
            .unwrap(),
    )
    .unwrap();
    h.app.close_pane_by_id(ephemeral);

    let old = fx
        .proj_dir
        .canonicalize()
        .unwrap()
        .join(".plexi/agent_memory/researcher");
    let legacy = serde_json::to_value(researcher_history_record(
        &fx.proj_dir,
        "legacy-claude",
        "claude",
    ))
    .unwrap();
    let assert_legacy_resume = |pane| {
        let argv = captured_argv(fx.home.path(), pane);
        let [settings_flag, settings_path, resume_flag, session_id] = argv.as_slice() else {
            panic!("expected one settings file and the legacy session ID: {argv:?}");
        };
        assert_eq!(settings_flag, "--settings");
        assert_eq!(resume_flag, "--resume");
        assert_eq!(session_id, "legacy-claude");
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(settings_path).unwrap()).unwrap();
        assert!(settings.get("autoMemoryDirectory").is_none());
        super::harness_tests::assert_claude_question_permission_hook(&settings);
    };
    for mut json in [expired, legacy] {
        json["claude_memory_dir"] = serde_json::json!(old);
        let record = serde_json::from_value(json).unwrap();
        let reply = h.app.reopen_agent_record(
            &record,
            None,
            None,
            crate::broker::pane_authority::GrantSource::OutsideProcess,
        );
        assert_eq!(reply["ok"], true, "{reply}");
        let pane = reply["pane_id"].as_u64().unwrap();
        assert_legacy_resume(pane);
        assert!(h.app.save_workspace_now());
        drop(h);
        h = HostHarness::from_profile_dir(&fx.profile_dir);
        assert_legacy_resume(pane);
        h.app.close_pane_by_id(pane);
    }
    assert!(!old.exists());
}

#[cfg(unix)]
#[test]
fn admission_global_identity_survives_shadow_reopen_eager_restore_and_standby_wake() {
    let fx = AgentFocusFixture::new();
    let _shell = capture_claude_argv(fx.home.path());
    let sessions = fx.home.path().join("claude-projects");
    std::fs::create_dir_all(sessions.join("-proj")).unwrap();
    let _sessions = crate::cli::agent::set_test_claude_projects_dir(sessions.clone());
    for id in ["eager-memory", "standby-memory"] {
        std::fs::write(sessions.join(format!("-proj/{id}.jsonl")), "{}\n").unwrap();
    }
    let config = fx.profile_dir.join("config.toml");
    std::fs::write(&config, "[agents]\nrestore_mode = 'eager'\n").unwrap();
    let mut h = HostHarness::from_profile_dir(&fx.profile_dir);
    let caller = h.add_focused_terminal();
    write_agent(&fx, "researcher", "claude");
    let eager = cli_launch(
        &mut h,
        &fx,
        caller,
        crate::cli::agent::agent_start_payload("researcher", &fx.proj_dir, None, &fx.stops())
            .unwrap(),
    );
    let standby = cli_launch(
        &mut h,
        &fx,
        caller,
        crate::cli::agent::agent_spawn_payload(
            "researcher",
            &fx.proj_dir,
            Some("background"),
            Some("background"),
            None,
            &fx.stops(),
        )
        .unwrap(),
    );
    let path = bound_path(&captured_argv(fx.home.path(), eager))
        .canonicalize()
        .unwrap();
    assert_eq!(path, memory_dir("researcher").canonicalize().unwrap());
    assert_eq!(
        bound_path(&captured_argv(fx.home.path(), standby))
            .canonicalize()
            .unwrap(),
        path
    );
    terminal_mut(&mut h, eager).agent_session_id = Some("eager-memory".into());
    terminal_mut(&mut h, standby).agent_session_id = Some("standby-memory".into());
    let session =
        serde_json::to_value(terminal(&h, standby).build_saved_agent_session().unwrap()).unwrap();
    let history = serde_json::to_value(
        terminal(&h, eager)
            .build_history_record(1, "test", fx.proj_dir.clone())
            .unwrap(),
    )
    .unwrap();
    shadow_researcher(&fx, &fx.proj_dir, "Later workspace persona");
    let record = serde_json::from_value(history.clone()).unwrap();
    let reply = h.app.reopen_agent_record(
        &record,
        None,
        None,
        crate::broker::pane_authority::GrantSource::OutsideProcess,
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let reopened = reply["pane_id"].as_u64().unwrap();
    let argv = captured_argv(fx.home.path(), reopened);
    assert_eq!(bound_path(&argv).canonicalize().unwrap(), path);
    assert_eq!(&argv[2..], ["--resume", "eager-memory"]);
    h.app.close_pane_by_id(reopened);
    assert!(h.app.save_workspace_now());
    drop(h);

    let mut h = HostHarness::from_profile_dir(&fx.profile_dir);
    for (pane, sid) in [(eager, "eager-memory"), (standby, "standby-memory")] {
        let argv = captured_argv(fx.home.path(), pane);
        assert_eq!(bound_path(&argv).canonicalize().unwrap(), path);
        assert_eq!(&argv[2..], ["--resume", sid]);
    }
    assert!(h.app.save_workspace_now());
    drop(h);

    std::fs::write(&config, "[agents]\nrestore_mode = 'lazy'\n").unwrap();
    let mut h = HostHarness::from_profile_dir(&fx.profile_dir);
    assert!(terminal(&h, standby).is_standby());
    let ready = fx.home.path().join(format!("ready-{standby}"));
    for _ in 0..100 {
        if ready.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        ready.exists(),
        "standby shell is ready for a full settings argument"
    );
    assert!(h.app.resume_standby_pane(standby).unwrap());
    let argv = captured_argv(fx.home.path(), standby);
    assert_eq!(bound_path(&argv).canonicalize().unwrap(), path);
    assert_eq!(&argv[2..], ["--resume", "standby-memory"]);
    let identity = serde_json::json!({"scope": "global", "agent_id": "researcher"});
    assert_eq!(history["claude_memory"], identity);
    assert_eq!(session["claude_memory"], identity);
    assert!(!fx.proj_dir.join(".plexi/agent_memory/researcher").exists());
}

#[test]
fn admission_saved_identity_decode_rejects_aliases_and_relative_workspace_roots() {
    let fx = AgentFocusFixture::new();
    let mut record =
        serde_json::to_value(researcher_history_record(&fx.proj_dir, "session", "claude")).unwrap();
    for binding in [
        serde_json::json!({"scope": "global", "agent_id": "researcher/"}),
        serde_json::json!({"scope": {"workspace_root": "../elsewhere"}, "agent_id": "researcher"}),
    ] {
        record["claude_memory"] = binding;
        assert!(
            serde_json::from_value::<crate::workspace::AgentHistoryRecord>(record.clone()).is_err()
        );
    }
}

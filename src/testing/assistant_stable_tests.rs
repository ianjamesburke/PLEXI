//! Stint 0752 (parked): tests for the Assistant on stable v1, bring-your-own-key.
//! Ignored until `ReleaseFeature::Assistant` moves to the stable tier.
//!
//! Every test runs under the `main` channel (a stable-tier channel) so the
//! release gate is the one a fresh install hits. Backends are test doubles:
//! the round trip scripts the model's tool call, and the no-backend case runs
//! the real `LiveAiBroker` against a key env var that is never set.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::app_trait::{App, ExternalTurnOpts};
use crate::plexi_ai::broker::LiveAiBroker;
use crate::testing::HostHarness;

fn stable_harness() -> (HostHarness, impl Drop, impl Drop, tempfile::TempDir) {
    let channel = crate::config::set_test_channel("main");
    let profile_dir = tempfile::tempdir().expect("profile dir");
    let profile = crate::config::set_test_profile_dir(profile_dir.path().to_path_buf());
    (HostHarness::new(), channel, profile, profile_dir)
}

fn open_assistant_from_palette(h: &mut HostHarness) -> u64 {
    let context_id = h.app.windows[h.app.active_window].context_id;
    assert!(
        h.app.locate_app_instance("assistant", Some(context_id)).is_none(),
        "no Assistant before the palette launch"
    );
    h.app.show_command_palette = true;
    h.app.sync_command_palette_focus();
    h.app.palette_query = "assistant".to_string();
    h.run_frames(2);
    h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    h.run_frames(2);
    let (_, pane_id, _) = h
        .app
        .locate_app_instance("assistant", Some(context_id))
        .expect("palette Enter on the Assistant entry must open the Assistant on stable");
    pane_id
}

fn submit(h: &mut HostHarness, assistant: u64, text: &str, reply: &Path) {
    App::submit_external_turn(
        h.assistant_mut(assistant),
        text.to_string(),
        format!("req-{}", uuid::Uuid::new_v4()),
        reply.display().to_string(),
        ExternalTurnOpts {
            conversation_id: None,
            join_desktop: true,
            status_for: None,
            client: None,
            kind: None,
        },
    )
    .expect("submit external turn");
}

fn wait_for_terminal_reply(h: &mut HostHarness, reply: &Path) -> serde_json::Value {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(reply) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if value["state"].as_str() != Some("waiting_for_permission")
                    && value["state"].as_str() != Some("running")
                {
                    return value;
                }
            }
        }
        h.run_frames(1);
        assert!(
            started.elapsed() < super::load_aware_timeout(Duration::from_secs(40)),
            "timed out waiting for the Assistant reply"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Palette -> Assistant -> `calc.add` through the real gated dispatcher.
/// The calc pane's `ExposeTools` registers its tools, the scripted model
/// issues the `ToolCall`, and the calc guest answers with a `ToolResult`.
#[test]
#[ignore = "parked: Assistant is not on stable (stint 0752); enable when the gate flips"]
fn stable_palette_opens_assistant_and_completes_calc_tool_round_trip() {
    let (mut h, _channel, _profile, _dir) = stable_harness();
    let calc = h.launch_repo_app("apps/calc", &[]);
    let assistant = open_assistant_from_palette(&mut h);
    assert_ne!(assistant, calc);

    let input = serde_json::json!({"a": 2.0, "b": 3.0});
    h.assistant_mut(assistant)
        .queue_scripted_tool("calc.add", &input.to_string());
    let reply = h.workspace_root().join("calc-reply.json");
    submit(&mut h, assistant, "add 2 and 3", &reply);
    let body = wait_for_terminal_reply(&mut h, &reply);
    let text = body.to_string();
    assert!(
        text.contains('5'),
        "calc.add 2+3 must come back through ToolResult: {text}"
    );
    assert!(body.get("error").map_or(true, |e| e.is_null()), "{text}");
}

/// No key, no backend: the stable Assistant surfaces the broker's existing
/// guidance as an error turn, never a blank reply or a beta-channel message.
#[test]
#[ignore = "parked: Assistant is not on stable (stint 0752); enable when the gate flips"]
fn stable_assistant_without_backend_shows_key_guidance_not_beta_message() {
    let (mut h, _channel, _profile, _dir) = stable_harness();
    let missing_env = "PLEXI_TEST_0752_OPENROUTER_API_KEY_MISSING";
    unsafe { std::env::remove_var(missing_env) };
    let mut ai = crate::config::AiConfig::default();
    ai.backend = Some("openrouter".to_string());
    ai.openrouter = Some(crate::config::OpenRouterBackendConfig {
        api_key_env: Some(missing_env.to_string()),
        tiers: Default::default(),
    });
    let assistant =
        h.add_assistant_pane_with_broker(0, Arc::new(LiveAiBroker::new(Some(ai))));
    let reply = h.workspace_root().join("nokey-reply.json");
    submit(&mut h, assistant, "hello", &reply);
    let body = wait_for_terminal_reply(&mut h, &reply);
    let text = body.to_string();
    assert!(text.contains("api_key_missing"), "{text}");
    assert!(text.contains("plexi secret set"), "{text}");
    assert!(
        !text.contains("beta") && !text.contains("not part of the stable"),
        "stable must not show a channel-gate message: {text}"
    );
}

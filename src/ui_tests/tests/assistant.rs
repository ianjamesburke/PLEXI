use super::*;

/// Host Assistant pane smoke: open → step → assert visible (epic Phase D).
#[test]
fn assistant_pane_opens_and_renders() {
    let ws = tempfile::tempdir().unwrap();
    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();
    h.open_assistant(ws.path().to_path_buf());
    h.run_steps(2);

    let assistant_panes = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .values()
            .filter(|p| {
                p.as_view()
                    .map(|a| a.runtime.type_id() == "assistant")
                    .unwrap_or(false)
            })
            .count()
    });
    assert_eq!(assistant_panes, 1, "assistant pane should be open");
    h.save_screenshot(&evidence_png!()).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!());
}

/// Visual review for stints 0466/0380: a tool row whose input/output carry
/// nested JSON strings must show decoded text (real newlines, real quotes)
/// rather than `\n` / `\"` literals, and a shared slash-command row must
/// render like an ordinary assistant row.
#[test]
fn screenshot_assistant_decoded_tool_rows_and_command_output() {
    use crate::assistant::model::{Turn, TurnRole};

    let ws = tempfile::tempdir().unwrap();
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);
    let row = |role: TurnRole, text: &str, at: &str| Turn {
        role,
        text: text.to_string(),
        created_at: at.to_string(),
        status: None,
        thoughts: None,
        detail: None,
        input_summary: None,
        output_preview: None,
    };
    let mut tool = row(TurnRole::Tool, "host.files.write", "2026-07-27T00:00:02Z");
    tool.status = Some(crate::assistant::model::ToolStatus::Succeeded);
    tool.input_summary = Some(
        r#"{path: notes/log.md, body: line one line two, note: he said "hi"}"#.to_string(),
    );
    tool.output_preview = Some("stdout: wrote 2 lines\nline one\nline two".to_string());

    assistant.model.turns = vec![
        row(TurnRole::User, "/context", "2026-07-27T00:00:00Z"),
        row(
            TurnRole::Command,
            "Assistant context:\n- Estimated transcript tokens: ~180 (4 turns)\n- Enabled tools: host.files.write, host.build.run",
            "2026-07-27T00:00:01Z",
        ),
        tool,
        row(
            TurnRole::Assistant,
            "Wrote both lines to `notes/log.md`.",
            "2026-07-27T00:00:03Z",
        ),
    ];

    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(3);
    let texts = painted_texts(&mut h);
    painted_rect(&texts, "Wrote both lines");
    painted_rect(&texts, "Assistant context");
    let escaped: Vec<_> = texts
        .iter()
        .filter(|(_, text)| text.contains("\\n") || text.contains("\\\""))
        .collect();
    assert!(
        escaped.is_empty(),
        "escape sequences painted literally: {escaped:?}"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Visual regression for stints 0435/0442: a real multi-turn conversation
/// (short + long messages on both sides) rendered end-to-end through the
/// actual assistant pane path, screenshotted for human/agent eyeball
/// review — not just the isolated `draw_turn_row` geometry assertions in
/// `assistant::render::tests`. Catches whole-conversation regressions
/// (e.g. every row rendering identically) that a single-turn unit test
/// cannot.
#[test]
fn screenshot_assistant_conversation_bubbles() {
    use crate::assistant::model::{Turn, TurnRole};

    let ws = tempfile::tempdir().unwrap();
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);
    assistant.model.turns = vec![
        Turn {
            role: TurnRole::User,
            text: "hi there big boii".to_string(),
            created_at: "2026-07-18T00:00:00Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        },
        Turn {
            role: TurnRole::Assistant,
            text: "Hey hey! What's up? 👋".to_string(),
            created_at: "2026-07-18T00:00:01Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        },
        Turn {
            role: TurnRole::User,
            text: "can you use markdown in your responses, just as an example so I can see."
                .to_string(),
            created_at: "2026-07-18T00:00:02Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        },
        Turn {
            role: TurnRole::Assistant,
            text: "Sure — here's **bold**, *italic*, and a list:\n\n- one\n- two\n- three"
                .to_string(),
            created_at: "2026-07-18T00:00:03Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        },
    ];

    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(3);
    // User bubbles hug the right edge and shrink to their text; assistant
    // rows start at the left. Identical rows would put both at one x.
    let texts = painted_texts(&mut h);
    let user = painted_rect(&texts, "hi there big boii");
    let assistant = painted_rect(&texts, "Hey hey!");
    assert!(
        user.min.x > assistant.min.x + 100.0 && user.max.x > assistant.max.x,
        "user bubble {user:?} must sit right of assistant row {assistant:?}"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Stint 0467, never-frozen rule: while a turn is in flight the
/// transcript always shows an animated element. Two screenshots: (1) the
/// model streamed a sentence and is now generating a tool call — the
/// generation row (`⠹ writing host.files.write · 3.4k chars`) renders
/// under the text; (2) same state with no generation info — the thinking
/// dots render under the text instead of the pre-0467 frozen bubble.
#[test]
fn screenshot_assistant_streaming_never_frozen() {
    use crate::assistant::model::{Turn, TurnRole};

    let ws = tempfile::tempdir().unwrap();
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);
    assistant.model.turns = vec![Turn {
        role: TurnRole::User,
        text: "build me a tictactoe plugin".to_string(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
        status: None,
        thoughts: None,
        detail: None,
        input_summary: None,
        output_preview: None,
    }];
    assistant.model.streaming.in_flight = true;
    assistant.model.streaming.partial_answer =
        "The SDK uses state.get() from plexi_sdk. Let me fix all the issues.".to_string();
    assistant
        .model
        .apply_tool_progress(Some("host.files.write".to_string()), 3400);

    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(3);
    // The generation row names the tool being written and its size.
    let texts = painted_texts(&mut h);
    painted_rect(&texts, "writing host.files.write · 3.4k chars");
    h.save_screenshot(&evidence_png!("_tool_progress"))
        .expect("render failed");

    // State 2: same turn with no generation info — a fresh pane, since
    // the harness has no post-open mutable assistant accessor. The dots
    // must take over under the text.
    let broker2: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant2 =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker2, ws.path(), 1);
    assistant2.model.streaming.in_flight = true;
    assistant2.model.streaming.partial_answer =
        "The SDK uses state.get() from plexi_sdk. Let me fix all the issues.".to_string();
    let mut h2 = PlexiUiHarness::new_sized(1000.0, 720.0);
    h2.step();
    h2.open_assistant_built(assistant2, ws.path().to_path_buf());
    h2.run_steps(2);
    // Without generation info nothing names a tool: the dots carry it.
    let texts = painted_texts(&mut h2);
    painted_rect(&texts, "Let me fix all the issues.");
    assert!(
        !texts
            .iter()
            .any(|(_, text)| text.contains("writing ") && text.contains(" chars")),
        "no generation row without tool progress: {texts:?}"
    );
    h2.save_screenshot(&evidence_png!("_dots_after_text"))
        .expect("render failed");
}

/// Stint 0455: a plugin-build turn's transcript renders chronologically —
/// user message, first assistant segment, individual caret-dropdown tool
/// rows (never a collapsed "N tool calls" trail), a failed row in the
/// danger hue, and the final reply as its own bubble below the tools.
/// Screenshot is for eyeball review of the caret rows and ordering.
#[test]
fn screenshot_assistant_transcript_with_tool_rows() {
    use crate::assistant::model::{ToolStatus, Turn, TurnRole};

    let ws = tempfile::tempdir().unwrap();
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);

    let mut write_tool = Turn::tool("host.files.write", ToolStatus::Succeeded);
    write_tool.input_summary = Some("path: main.py".to_string());
    write_tool.output_preview = Some("{\n  \"ok\": true,\n  \"bytes\": 512\n}".to_string());
    write_tool.detail = Some(
        "--- a/main.py\n+++ b/main.py\n@@ -1,1 +1,2 @@\n-pass\n+print(\"hi\")\n".to_string(),
    );
    let mut check_tool = Turn::tool("plexi.plugin_check", ToolStatus::Succeeded);
    check_tool.input_summary = Some(r#"{"plugin": "guessing-game"}"#.to_string());
    check_tool.output_preview = Some(r#"{"ok": true}"#.to_string());
    let mut failed_tool = Turn::tool("host.build.run — exit 1: mypy error", ToolStatus::Failed);
    failed_tool.input_summary = Some(r#"{"cmd": "plugin check"}"#.to_string());
    // Multi-line output preview (stint 0460): real newlines render as a
    // framed block, so a command's output reads as the lines it printed.
    failed_tool.output_preview = Some(
        "stdout: ✓ manifest — fish-game\n✗ types — mypy found errors:\nmain.py:12: error: \"UiValueChange\" has no attribute \"payload\"\n✗ plugin check failed — 1 error(s)".to_string(),
    );

    assistant.model.turns = vec![
        Turn::now(
            TurnRole::User,
            "Build a simple number guessing game as a Plexi plugin.",
        ),
        Turn::now(
            TurnRole::Assistant,
            "Let me build this for you. Starting with scaffolding the plugin.",
        ),
        write_tool,
        check_tool,
        failed_tool,
        Turn::now(
            TurnRole::Assistant,
            "The Guessing Game plugin is open! Type a number and press Enter.",
        ),
    ];
    // An in-flight tool call so the animated running row (spinner +
    // elapsed seconds, stint 0460) is on the screenshot too.
    assistant.model.streaming.in_flight = true;
    assistant.model.tool_call_started(
        "host.build.run",
        r#"{"args": ["plugin", "check", "fish-game"]}"#,
    );

    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(3);
    // Chronological: first segment, each tool row, then the final reply.
    let texts = painted_texts(&mut h);
    let tops: Vec<f32> = [
        "Starting with scaffolding the plugin.",
        "host.files.write",
        "plexi.plugin_check",
        "host.build.run — exit 1",
        "The Guessing Game plugin is open!",
    ]
    .iter()
    .map(|needle| painted_rect(&texts, needle).min.y)
    .collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "transcript rows out of order: {tops:?}"
    );
    assert!(
        !texts.iter().any(|(_, text)| text.contains("tool calls")),
        "tool rows must never collapse into a summary: {texts:?}"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Stint 0451: emoji in an assistant reply must rasterize as monochrome
/// glyphs, not tofu boxes. Before the Noto Emoji fallback was bound in
/// `theme::font_definitions`, none of the bundled fonts covered the emoji
/// planes, so these codepoints drew as empty boxes. Screenshot is for
/// eyeball review — confirm each emoji draws a distinct shape.
#[test]
fn screenshot_assistant_emoji_render() {
    use crate::assistant::model::{Turn, TurnRole};

    let ws = tempfile::tempdir().unwrap();
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);
    assistant.model.turns = vec![
        Turn {
            role: TurnRole::User,
            text: "show me some emoji".to_string(),
            created_at: "2026-07-18T00:00:00Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        },
        Turn {
            role: TurnRole::Assistant,
            // Exact transcript codepoints from stint 0451: U+1F44B U+1F604
            // U+1F6D2 U+2728 U+1F3AE.
            text: "Here you go: 👋 😄 🛒 ✨ 🎮".to_string(),
            created_at: "2026-07-18T00:00:01Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        },
    ];

    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(3);
    // Tofu means no bound face covers the codepoint; the fallback chain
    // must cover every one at the transcript's proportional face.
    let covered = h.with_app(|app| {
        app.ctx.fonts_mut(|fonts| {
            fonts.has_glyphs(&egui::FontId::proportional(14.0), "👋😄🛒✨🎮")
        })
    });
    assert!(
        covered,
        "an emoji in the reply has no glyph in the font chain"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

#[test]
fn semantic_label_lookup_is_scoped_to_the_target_pane() {
    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    let workspace = h.workspace_root().to_path_buf();
    let assistant = h
        .open_target(HarnessOpenTarget::Builtin {
            id: "assistant",
            cwd: &workspace,
            args: &[],
        })
        .expect("open Assistant");
    h.step();
    let files = h
        .open_target(HarnessOpenTarget::Builtin {
            id: "file_browser",
            cwd: &workspace,
            args: &[],
        })
        .expect("open File Browser");
    h.run_steps(2);

    // Stint 0424 removed the header bar (which used to carry "Assistant"/
    // the session name); the default footer hint bar's "commands" label
    // is the stable, assistant-only text left to scope against.
    assert!(h.pane_has_label(assistant, "commands"));
    assert!(!h.pane_has_label(files, "commands"));
}

#[test]
fn native_builtin_pane_state_retains_committed_semantics() {
    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    let workspace = h.workspace_root().to_path_buf();
    let assistant = h
        .open_target(HarnessOpenTarget::Builtin {
            id: "assistant",
            cwd: &workspace,
            args: &[],
        })
        .expect("open Assistant");
    let files = h
        .open_target(HarnessOpenTarget::Builtin {
            id: "file_browser",
            cwd: &workspace,
            args: &[],
        })
        .expect("open File Browser");
    h.run_steps(2);

    let (assistant_state, files_state) = h.with_app(|host| {
        let state = |pane_id| {
            host.windows
                .iter()
                .find_map(|window| window.panes.get(&pane_id))
                .and_then(Pane::as_view)
                .map(|pane| pane.semantic_state())
                .expect("builtin pane state")
        };
        (state(assistant), state(files))
    });

    assert_eq!(assistant_state.runtime_kind, "builtin");
    // Stint 0424 removed the header bar (which used to carry "Assistant"
    // as a committed semantic node); the footer hint bar's "commands"
    // label is the stable text left to check committed-semantics against.
    assert!(assistant_state.nodes.iter().any(|node| {
        node.label.as_deref() == Some("commands") || node.value.as_deref() == Some("commands")
    }));
    assert_eq!(files_state.runtime_kind, "builtin");
    assert!(!files_state.nodes.is_empty());
}

/// Assistant pane with a pending permission sheet renders (Phase D2).
#[test]
fn assistant_permission_sheet_renders() {
    let ws = tempfile::tempdir().unwrap();
    let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
    h.step();

    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);
    assistant
        .model
        .permission_requested("csv.write_range", r#"{"range": "A1:B2"}"#);
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(2);

    let assistant_panes = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .values()
            .filter(|p| {
                p.as_view()
                    .map(|a| a.runtime.type_id() == "assistant")
                    .unwrap_or(false)
            })
            .count()
    });
    assert_eq!(assistant_panes, 1, "assistant pane should be open");
    h.save_screenshot(&evidence_png!()).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!());
}

/// Seeded transcript (user bubble right, assistant plain left) with the
/// slash-command picker open above the bottom-pinned composer.
#[test]
fn assistant_transcript_and_picker_render() {
    let ws = tempfile::tempdir().unwrap();
    let mut h = PlexiUiHarness::new_sized(1280.0, 900.0);
    h.step();

    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    let mut assistant =
        crate::assistant::AssistantView::new(ws.path().to_path_buf(), broker, ws.path(), 1);
    let turn = |role, text: &str| crate::assistant::model::Turn {
        role,
        text: text.to_string(),
        created_at: "2026-06-12T00:00:00Z".to_string(),
        status: None,
        thoughts: None,
        detail: None,
        input_summary: None,
        output_preview: None,
    };
    assistant.model.turns = vec![
        turn(
            crate::assistant::model::TurnRole::User,
            "Summarize the open PRs for this repo, please.",
        ),
        turn(
            crate::assistant::model::TurnRole::Assistant,
            "There are **3 open PRs**:\n\n\
                 - [#2224](https://github.com/x/x/pull/2224) overhauls the assistant UI\n\
                 - #2220 fixes `IPC` socket recovery\n\
                 - #2218 regenerates docs",
        ),
        turn(crate::assistant::model::TurnRole::User, "Thanks!"),
        // A tool row carrying a file-edit diff payload (stint 0421) —
        // exercises draw_diff_block through the real transcript path.
        crate::assistant::model::Turn {
            status: Some(crate::assistant::model::ToolStatus::Succeeded),
            detail: Some(
                "--- a/main.py\n+++ b/main.py\n@@ -1,2 +1,2 @@\n-count = 0\n+count = 5\n print(count)\n"
                    .to_string(),
            ),
            ..turn(crate::assistant::model::TurnRole::Tool, "host.files.edit")
        },
    ];
    assistant.model.composer = "/".to_string();
    h.open_assistant_built(assistant, ws.path().to_path_buf());
    h.run_steps(10);
    // A bare "/" opens the command picker over the seeded transcript,
    // listing the built-in commands as ListRows.
    let texts = painted_texts(&mut h);
    painted_rect(&texts, "/help");
    assert_layout_surface(&h, "list");
    painted_rect(&texts, "Summarize the open PRs for this repo, please.");
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

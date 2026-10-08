Status: active
Stint: none yet

# Pi-style assistant harness

Design note for a native Rust agent loop behind `[ai] harness`. The Assistant's default loop stays the current one. `"pi"` selects this harness. Nothing here is copied from Pi's TypeScript.

## Credit and license

[Pi](https://github.com/badlogic/pi-mono) is Mario Zechner's (badlogic / earendil-works) minimal coding-agent harness. The monorepo publishes `@earendil-works/pi-agent-core` (loop, messages, tools, steering) and `@earendil-works/pi-coding-agent` (sessions, compaction, extensions). Both are **MIT** licensed. This document and the Rust module `pi_harness` restate the design in Plexi's own types. They do not vendor Pi source.

## What Pi's core does

Pi splits a low-level loop from a session harness.

The loop owns one run. Each turn is: prepare the provider request, stream one assistant message, execute that message's tool calls, then decide whether to continue. A tool call is never left without a tool-result message. If the model stops because the reply was truncated, every call in that message is failed instead of executed. If the run is aborted, the assistant message is kept with an aborted stop reason and the loop exits. Tool errors are results the model sees (`isError`), not thrown failures that kill the process.

Messages are an agent transcript, not the provider payload. Roles the model understands are user, assistant, and tool result. Anything else (UI notes, compaction markers, notifications) stays in the transcript and is filtered or rewritten by a convert step immediately before the provider call. A transform step in front of that can prune or inject context. The next request is built from the transcript, not from a side channel that dropped tool rows.

Compaction is a structural session edit, not a silent trim of the provider array. Pi walks backward from the newest entry until a recent-token budget is filled, summarizes the older span (feeding the previous summary back in), and appends a compaction entry that names the first entry kept verbatim. The next request is the summary plus that tail. An extension hook (`session_before_compact`) can cancel compaction or supply the summary. Recent tail entries stay verbatim.

Extensions register tools and hooks. The hooks that bound a turn are: transform or prepare-request (context), before-tool (may block with a reason the model sees as an error result), after-tool (may replace the result), and a turn-finalizer that can stop or force another provider call. Hook order is registration order. Subscribers are awaited in that same order.

Sessions are an append-only entry log (messages, compaction, branch summaries) that can be reloaded. Reloading rebuilds the same provider context the uninterrupted session would build. Steering messages are injected after the current assistant message's tools finish, before the next provider call. Follow-up messages are injected only when the model would otherwise stop. `abort` cancels the in-flight provider stream and tool work and leaves the transcript consistent.

## Where that maps in Plexi

| Pi idea | Plexi today | This harness |
|---|---|---|
| Provider call | `plexi_ai` broker (`LiveAiBroker`) and backends `openrouter`, `ollama`, `local` | One provider call per step via `AiBroker::complete_once`. The broker still writes its ledger row and still selects the backend. |
| Tool loop inside the broker | `run_turn_and_respond` dispatches tools and appends JSON tool results on a private array | Not used for the Pi path. `single_completion` returns the model's tool calls and does not dispatch them. |
| Transcript | `Turn` rows. `history_messages` sends user and assistant text and drops `TurnRole::Tool`, so a later turn can lose a file read | `SessionEntry` log. Every assistant tool call is followed by its tool result in the next provider request. |
| Tools | `ToolDispatcher` and `assistant_host_tools` | Eval tools are handlers on the harness. Host tools stay the production backend; wiring them into this loop is a follow-up. |
| Permission | Grant store, permission monitor, Assistant ask sheet | A tool that needs a grant emits a permission-ask and waits. Allow-always is a `GrantRecord` in the existing grant store. Deny is a structured tool error. |
| Compaction | `/compact` checkpoints on `AssistantStore` | Before a request that exceeds the token budget, older turns are summarized. Recent user turns stay verbatim. `on_compact` can replace the summary. |
| Extensions | Skills and slash commands | `PiPlugin`: `before_turn`, `before_tool`, `after_tool`, `on_compact`, `on_turn_end`, plus optional tools and a context block. |
| Open panes | Broker prepends a short pane list (`PaneContext` is type id and pane id) | `OpenPanePlugin` injects app type, title, path, focus, and a bounded excerpt each turn. |
| Sessions | `conversations/<id>.jsonl`, one `Turn` per line | Successor file `conversations/<id>.pi.jsonl` in the same assistant directory. User lines use the same `role` / `text` / `created_at` fields as `Turn`, so the existing loader reads them. Assistant lines that record tool calls or a model id, tool lines, and compaction lines are skipped by that loader. Resume reads this file. |
| Ledger | `LedgerRow.model` is the configured slug the broker resolved before the call | The harness appends a row whose `model` is the id the stepper says was served. `ModelVisibility::picker_label` is `Medium — <served id>` for a later picker. This PR does not change the picker UI. |
| Abort | `CancelToken` on the current broker loop | The same token, checked between the provider call and each tool. An abort after a tool call appends an aborted tool result so the pair stays intact. |
| Steering | Not a first-class queue | `steer` appends a user entry after the current tool batch, before the next provider call. The composer does not feed that queue yet. |

## What it replaces, and what it reuses

When `[ai] harness` is unset or `"current"`, `AssistantApp::start_turn` is unchanged.

When it is `"pi"`, that method runs this loop instead of the broker's internal tool loop. The call still goes through `LiveAiBroker` (or whichever `AiBroker` the pane holds) for openrouter, ollama, and local. The grant store, ledger, and cancel token are the existing ones. Host-tool execution and the permission sheet are not on this path yet: a Pi turn can answer, stream text, compact, and persist its own session, and it does not dispatch `host.files.*`. That bridge is a follow-up, so dogfood of the flag is the loop and the transcript file, not a full swap of host tools.

`history_messages` is the bug this loop is meant to retire. It is still what the default harness sends.

## Config

```toml
[ai]
# harness = "current"   # default. "pi" selects this loop.
```

An unknown value logs a warning and keeps the current loop.

## Model slug versus served id

The configured slug is what we send (`model_medium = "xiaomi/mimo-v2.5"`, or a longer alias such as `xiaomi/mimo-v2.5-pro`). The broker ledger row stores that requested id. It does not read the `model` field the provider echoes in the response body. OpenRouter can serve a canonical id that is not the slug: an alias, a dated snapshot, or a route that pins `xiaomi/mimo-v2.5` when the config says `xiaomi/mimo-v2.5-pro`. Those two strings differ because one is the request and one is the provider's id, and the broker only persists the request.

The Pi stepper reports the served id on each step. The harness ledger row uses that served id. `ModelVisibility` keeps both `configured_model` and `served_model`, and `picker_label` uses the served id (`Medium — xiaomi/mimo-v2.5`). Parsing the echoed id inside `OpenRouterBackend` is a follow-up; until then a broker-backed step that does not override the id records the configured slug as the served id.

## Compaction and stale reads

Token estimates are character length divided by four, the same rough count Pi uses. Over budget, the last `keep_recent_turns` user turns (a user entry plus the assistant and tool entries that belong to it) stay verbatim. Older projected entries are replaced by one summary. The default summary is extractive and truncated so the prompt shrinks without another model call. A plugin that returns a summary from `on_compact` replaces that strategy. The event is `HarnessEvent::Compaction` and an info log.

A `host.files.read` result stores the path and mtime. The next request marks that result stale and appends a note when the mtime changes, or when the user asks to read that path again. The note tells the model to call the read tool. The note is rebuilt on each request, so a resumed session with the same disk state produces the same request.

## Follow-ups

Host tools are not registered on the Pi turn. `register_tool`, the allow-once and allow-always decisions, and a before-tool veto are compiled for the evals; the shipped binary's decider stays deny-only until a production caller constructs those decisions and registers the host tools. The pane still renders the current transcript and does not show Pi tool rows. The `/model` picker does not read `picker_label`. Composer steering, parallel tool batches, and model-written compaction are not wired. The OpenRouter response's echoed model id is not parsed into the broker ledger row. A live temp.md replay is an ignored test behind `PLEXI_LIVE_EVALS=1` and `OPENROUTER_API_KEY`. The Assistant pane UI pass is a separate change.

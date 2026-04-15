# Chat Interface Primitive

**Status:** Draft (vision spec)
**Last updated:** 2026-04-11
**Related specs:** [intelligence-protocol.md](../subsystems/intelligence-protocol.md) (deferred), [agent-mode.md](../subsystems/agent-mode.md), [agent-replay-testing.md](agent-replay-testing.md), [core-advanced-ui-sdk.md](core-advanced-ui-sdk.md)
**Related issues/PRs:** #93 (closed, intelligence proxy), #108 (agent-mode LLM backend), #85 (agent-controlled app UI)

---

## 1. Motivation

Plexi's draw protocol has been climbing the abstraction ladder. It started with `rect` / `text` / `line`, added `list`, and is now rolling out `image`, `video_thumbnail`, `file_grid`, `drop_target`, and `notification`. Each rung of the ladder trades generality for "write less code to ship a polished app."

The next obvious rung is **chat**. A meaningful share of the apps on Plexi's near-term roadmap — Parallax, GitHub Issues, Backlog Triage, Aquarium, Focus Manager, the terminal's own agent mode — all need a conversation UI. Today each one would reinvent:

- Message bubbles / lines with role styling
- Scroll-to-bottom behavior with sticky bottom when the user is already there
- Streaming partial tokens without layout thrash
- Markdown rendering (bold, italics, inline code, lists, links)
- Code blocks with language headers and copy buttons
- Tool-call / tool-result rendering
- An input box with multi-line editing, submit affordance, and history navigation
- Regenerate / edit / branch controls

This is the same "each app reinvents a file grid" problem `FileGrid` solved. The right answer is the same: ship a high-level primitive once, in Rust, and let every app instantiate it with a callback.

**Core pitch.** An app writes `ctx.chat(x, y, w, h, messages=[...], on_send=cb)` and gets a polished chat UI — streaming, markdown, tool calls, the lot — with the app still owning the conversation state and deciding how to produce assistant replies.

---

## 2. Prior Art

The chat-UI design space is well-explored. This spec leans on four sources explicitly and steals the learnings that matter for a terminal-multiplexer primitive.

### 2.1 T3 Chat (t3.chat)

- **Stack:** React + Next.js + TypeScript, Dexie (IndexedDB) for local-first storage, Vercel AI SDK for streaming, Tailwind, React Compiler to cut re-renders, the Marked lexer for incremental markdown parsing. Source: [Grokipedia — T3 Chat](https://grokipedia.com/page/T3_Chat), [Theo Browne — T3 Chat redesign](https://www.linkedin.com/posts/t3gg_t3-chat-is-now-the-cheapest-fastest-and-activity-7307892576580161536-eYBv).
- **What it does uniquely well:**
  - **Speed as a feature.** They optimistically render the user's message, stream from the server, and route through the fastest inference provider they can reach. The claimed "2× faster than ChatGPT" comes from attacking every stall point — local-first state, no server round-trip for navigation, markdown chunking so re-renders stay cheap during streaming.
  - **Resumable streams.** A token stream survives a page refresh — the server keeps the stream alive and the client re-attaches. Relevant pattern even if Plexi's pipe is local-process.
  - **Branching.** Any message can fork a new conversation thread. The data model is a tree, not a flat list. The UI lets you jump between siblings.
  - **Multi-model switching.** One conversation, swap the model mid-thread. Backed by a unified provider abstraction.
- **Related open-source clones:** [thom-chat](https://github.com/TGlide/thom-chat), [NOT-T3-Chat](https://github.com/Hairetsu/NOT-T3-Chat), [shaltielshmid/NotT3Chat](https://github.com/shaltielshmid/NotT3Chat), and the 2025 [T3 Cloneathon](https://github.com/t3-oss). T3 Chat itself is not open source but the patterns are well-documented.
- **Learning for Plexi:** Tree-structured conversation state > flat array, from day one. Streaming must be cheap to re-render (incremental markdown, not re-parse-from-scratch). The user's "what's fast" intuition is shaped by frame-to-first-token latency, not total tokens/sec — so the primitive must render the user's message and a "thinking" indicator before the app's callback even returns.

### 2.2 Vercel AI SDK

- **What it is:** A free, open-source TypeScript toolkit ([github.com/vercel/ai](https://github.com/vercel/ai), [ai-sdk.dev](https://ai-sdk.dev/docs/introduction)) that provides (a) a unified provider abstraction across OpenAI/Anthropic/Google/etc., (b) `useChat`/`useCompletion` React hooks, (c) a standardized streaming protocol, and (d) generative UI via RSC.
- **`useChat` semantics.** Manages the full message list, appends user messages, handles streaming assistant responses, and maintains history automatically. The app provides an endpoint; the hook handles everything else.
- **Stream protocol (SDK v5+).** SSE-based UI-message stream with typed parts: `text-start` / `text-delta` / `text-end` for text blocks (with unique IDs so deltas merge correctly), plus typed `tool-call` / `tool-result` / `reasoning` / `file` / `data-*` parts for custom structured data. Requires the `x-vercel-ai-ui-message-stream: v1` header. Source: [AI SDK UI: Stream Protocols](https://ai-sdk.dev/docs/ai-sdk-ui/stream-protocol), [AI SDK 5 announcement](https://vercel.com/blog/ai-sdk-5), [AI SDK v5 Internals — Part 4](https://dev.to/yigit-konur/vercel-ai-sdk-v5-internals-part-4-decoupling-client-server-state-management-and-message-1lb1).
- **Tool-call UI.** Tool calls are first-class message parts. The UI renders them inline (collapsible, with the call arguments and eventual result visible), not as separate turns. Source: [Multi-Step & Generative UI — Vercel Academy](https://vercel.com/academy/ai-sdk/multi-step-and-generative-ui).
- **Generative UI (RSC).** The LLM can stream React Server Components — the model emits a component descriptor, the server renders it, the client displays it. Lets the LLM return a weather widget or a chart instead of text. Interesting but not portable to a Rust/JSON protocol.
- **Pricing.** The SDK itself is MIT-licensed and free. The optional Vercel AI Gateway (default routing backend) has its own pricing but is not required.
- **Learning for Plexi:** The typed-parts streaming protocol is the right shape — Plexi's primitive should accept parts (`text`, `tool_call`, `tool_result`, `image`, `code`) rather than a single string, so apps and the LLM can stream structured content. Don't invent this from scratch — adopt Vercel's shape directly so apps that already speak it (or use its TS schema) port easily.

### 2.3 OpenRouter

- **What it is:** A unified API gateway across 290+ models from every major provider, OpenAI-compatible API, one key. Source: [openrouter.ai](https://openrouter.ai/), [OpenRouter Review 2025](https://skywork.ai/blog/openrouter-review-2025-unified-ai-model-api-pricing-privacy/).
- **Pricing model.** Per-token, passed through at or near direct-provider cost. No per-request markup on most models. Source: [OpenRouter Pricing](https://openrouter.ai/pricing).
- **Prompt caching.** Cache reads at 0.25×–0.50× the input-token price. **Caveat:** automatic `cache_control` routing works when requests are pinned to the Anthropic provider directly; if OpenRouter falls back to another provider mid-session, caching breaks. Source: [OpenRouter Prompt Caching docs](https://openrouter.ai/docs/guides/best-practices/prompt-caching), [open GitHub issue on caching regressions](https://github.com/OpenRouterTeam/ai-sdk-provider/issues/35).
- **Provider routing.** Can pin a provider, or let OpenRouter pick based on cost/latency/availability. Source: [Provider Routing docs](https://openrouter.ai/docs/guides/routing/provider-selection).
- **Learning for Plexi:** OpenRouter is attractive as a *default* backend for a Plexi-hosted chat primitive — one key, everything works, model switching is a parameter change. But pinning to a single provider for cache-heavy workloads is essential; auto-routing defeats Anthropic's prompt cache. If Plexi ever adopts routing, the default should be `provider=anthropic` when using Claude models so caching isn't silently lost.

### 2.4 Cline / aider / Continue.dev / Cursor

- **Cline** ([cline/cline](https://github.com/cline/cline)): VS Code extension with a Plan/Act separation — read-only planning turn then execution turn. Rich streaming + human-in-the-loop tool approval. Source: [Cline — AI Coding, Open Source and Uncompromised](https://cline.bot), [Cline docs](https://docs.cline.bot/api/chat-completions).
- **Continue.dev CLI** ([continuedev/continue](https://deepwiki.com/continuedev/continue/10-cli-tool)): React-based terminal UI via Ink, streams LLM output through a TUI chat. Tool system with `readFileTool` / `writeFileTool` / etc. Closest existing cousin to what a Plexi-hosted chat primitive would look like in a terminal.
- **aider** ([aider.chat](https://aider.chat/)): Git-native CLI chat. No UI framework — just terminal text with ANSI colors. Pragmatic rather than beautiful. Proves you can ship a useful chat UX with very little chrome.
- **Cursor:** Closed-source, but the interesting UX primitive is inline diff-apply preview for code-block messages and the "apply-in-editor" button. Not directly portable to a generic primitive.
- **Learning for Plexi:** Terminal chat UIs work fine without React. The important rendering primitives are (a) wrapped text with scroll, (b) syntax-highlighted code blocks, (c) collapsible tool-call blocks, (d) an input line. Plexi already has (a). Code highlighting and collapsible blocks are where the primitive earns its keep.

---

## 3. Scope Decision

The single biggest design question: **does `ctx.chat(...)` include LLM routing, or only rendering?**

### Options

- **A. Pure rendering.** App provides `messages`, app handles streaming-in deltas via a `chat_append_delta(chat_id, part)` command, app owns LLM calls. Plexi only draws.
- **B. Rendering + routing.** App provides `system`, `tools`, and maybe a `provider` hint. Plexi resolves the model, makes the call, streams deltas into the primitive's state automatically, surfaces tool calls to the app via events, and reports cost.
- **C. Both modes.** Same primitive; opt-in via a `provider` field. Absent → pure rendering (app drives). Present → Plexi drives.

### Prior architectural context

Plexi deliberately **rejected** a centralized intelligence proxy ([intelligence-protocol.md](../subsystems/intelligence-protocol.md), issue #93). Apps currently manage their own LLM calls: declare required secrets in `manifest.toml`, resolve them via `SecretGet`, call providers themselves, and report costs back via `cost_report`. The architectural reason was sound — keeping Plexi out of the LLM business means no async-handler refactor, no provider-API maintenance burden, no inline cost-enforcement code in the render path.

PR #108 (agent-mode LLM backend) partially contradicts this: for *agent mode specifically*, Plexi does make the Anthropic API call directly, through an `LlmWorker` thread with ureq. That was justified because agent mode is Plexi's own feature, not a third-party app. The worker pattern is a proof point that Plexi *can* host LLM calls cleanly when it wants to.

### Recommendation: **Option A — pure rendering, v1. Revisit routing in v2 once two apps ship on the primitive.**

Reasoning:

1. **The thing we are sure about is UI, not routing.** Every app wanting a chat UI today already knows how to call an LLM. What they are reinventing is message rendering, streaming layout, markdown, code blocks, and tool-call display. That's where the primitive earns its keep. Mixing routing in bloats v1 and re-opens the intelligence-proxy architectural debate that was explicitly closed.

2. **Routing is cheap to add later, expensive to subtract.** If v1 is rendering-only and v2 adds an opt-in `provider` field, no existing caller breaks. If v1 bundles routing and we later want to strip it (e.g., because provider-API maintenance becomes a burden, or because Plexi goes WASM and can't make outbound HTTP calls), every caller has to be migrated.

3. **The PR #108 worker is the model, not the primitive.** Agent mode's `LlmWorker` is a *Plexi-internal* feature that happens to use an LLM. It should not be exposed to third-party apps through the chat primitive. When v2 of this spec revisits routing, the right move is to generalize PR #108's worker into a reusable `ProviderWorker` and wire it under the primitive — but only after agent mode has proven the pattern in production.

4. **The current cost-reporting architecture already works.** Apps make calls, report via `cost_report`, Plexi aggregates. Adding centralized routing duplicates that pipeline rather than replacing it.

5. **v2 is a smaller ask if v1 is right.** Once `ctx.chat` is the standard way to draw a chat, adding `provider="openrouter:anthropic/claude-sonnet-4.5"` is literally one new field and one new worker hookup. The user can decide to revisit the intelligence-proxy question with real usage data in hand.

**What this means concretely for v1:**

- `ctx.chat(...)` takes `messages`, draws them, emits `ChatSubmit` events when the user hits enter, and accepts streaming deltas via a follow-up draw command.
- The app is responsible for calling the LLM and feeding deltas back.
- No `provider` / `model` / `tools` routing parameters in v1.
- `tools` *is* a v1 concept — but only for **rendering** tool calls that the app produces. The app still decides which tools exist and executes them.

---

## 4. API Shape

Target: match the existing `DrawCommand` style — flat, JSON-serializable, ID-keyed when stateful.

### 4.1 JSON wire format (authoritative)

```json
{
  "type": "chat",
  "chat_id": "main",
  "x": 0, "y": 0, "w": 800, "h": 600,
  "messages": [
    {"id": "m1", "role": "user",      "parts": [{"type": "text", "text": "explain ffmpeg concat"}]},
    {"id": "m2", "role": "assistant", "parts": [
      {"type": "text", "text": "The concat demuxer..."},
      {"type": "code", "language": "bash", "text": "ffmpeg -f concat -i list.txt -c copy out.mp4"}
    ]}
  ],
  "streaming": {"message_id": "m3", "done": false},
  "style": "bubbles",
  "placeholder": "Ask anything..."
}
```

### 4.2 Python SDK sketch

```python
ctx.chat(
    chat_id="main",
    x=0, y=0, w=pane_w, h=pane_h - 40,
    messages=state["messages"],
    style="bubbles",              # "bubbles" | "lines" | "compact"
    placeholder="Ask anything...",
    streaming=state.get("streaming"),  # {"message_id": "m3", "done": False} or None
)
```

On submit, Plexi emits an event to the app:

```json
{"type": "chat_submit", "chat_id": "main", "text": "explain ffmpeg concat"}
```

The app handles it in `on_event`, appends to `messages`, kicks off its LLM call, and starts feeding deltas:

```python
def on_chat_submit(ev):
    state["messages"].append({"id": new_id(), "role": "user", "parts": [{"type": "text", "text": ev["text"]}]})
    assistant_id = new_id()
    state["messages"].append({"id": assistant_id, "role": "assistant", "parts": []})
    state["streaming"] = {"message_id": assistant_id, "done": False}
    start_llm_call(assistant_id)  # app's own worker thread
```

To stream, the app emits incremental append commands during frames:

```python
ctx.chat_append_delta(
    chat_id="main",
    message_id=assistant_id,
    part={"type": "text", "text": delta},  # appended to most recent text part, or starts a new one
)
# when done:
ctx.chat_stream_end(chat_id="main", message_id=assistant_id)
```

### 4.3 Rust SDK sketch

```rust
ctx.chat(ChatParams {
    chat_id: "main".into(),
    rect: Rect::from_xywh(0.0, 0.0, pane_w, pane_h - 40.0),
    messages: &state.messages,
    style: ChatStyle::Bubbles,
    placeholder: Some("Ask anything...".into()),
    streaming: state.streaming.as_ref(),
});
```

### 4.4 Message part schema

```json
{"type": "text",        "text": "..."}
{"type": "code",        "language": "rust", "text": "..."}
{"type": "tool_call",   "id": "tc_1", "name": "read_file", "input": {"path": "..."}}
{"type": "tool_result", "tool_call_id": "tc_1", "output": "..."}
{"type": "image",       "path": "stills/scene_01.png"}
{"type": "reasoning",   "text": "..."}   // collapsed by default; matches Anthropic's thinking blocks
```

Shape is deliberately close to Anthropic's messages API and Vercel AI SDK's UI-message parts — apps already speaking either format should need near-zero translation.

### 4.5 Events from Plexi to app

| Event | Fields | When |
|---|---|---|
| `chat_submit` | `chat_id`, `text` | User pressed Enter |
| `chat_regenerate` | `chat_id`, `message_id` | User clicked regenerate on an assistant message |
| `chat_edit` | `chat_id`, `message_id`, `new_text` | User edited a user message |
| `chat_branch` | `chat_id`, `from_message_id` | User forked a new branch (v2) |
| `chat_copy` | `chat_id`, `message_id` | User clicked copy — Plexi also copies to clipboard, this is just a hook |

---

## 5. Features — v1 / v2 / Deferred

### v1 (ships first)

- Message list with role-based styling (user / assistant / system)
- Two built-in styles: `bubbles` (rounded cards) and `lines` (indent-based, terminal-native)
- Text parts with inline markdown (bold, italic, inline code, links)
- Code parts with language label; monospace font; copy button
- Tool-call parts as collapsible inline blocks (name + input + result)
- Input box at bottom with multi-line support (Shift+Enter for newline)
- Auto-scroll-to-bottom when user is already at bottom; sticky when user has scrolled up
- Streaming via `chat_append_delta` — Plexi handles the append-to-last-text-part merging
- `chat_submit` / `chat_regenerate` / `chat_edit` events
- "Thinking..." indicator while `streaming.done == false` and no content yet

### v2 (after v1 ships and two apps adopt it)

- Conversation branching (`chat_branch` event + tree-structured `messages`)
- Model-picker chrome (purely visual — app wires it to its own provider switch)
- Optional `provider:` field for Plexi-routed LLM calls (revisits the intelligence-protocol decision with real data)
- Image parts rendered inline
- Reasoning parts (collapsed by default, expandable)
- Message-level reactions / annotations for use in agent-replay feedback loops
- `compact` style (single-line wrapped, no bubbles) for dense telemetry chats
- Multiple chats in one pane (primary use: parallel agent conversations)

### Explicitly deferred

- Syntax highlighting beyond one color per language (needs a highlighter dependency — probably `tree-sitter-highlight` or `syntect`; defer until a code-heavy app complains)
- RSC-style generative UI (the LLM cannot emit Rust/egui components over JSON; this is a React-only affordance and not portable)
- Resumable streams across Plexi restarts (the JSON pipe dies with the app process; resumability would be a session-persistence feature, not a chat feature)
- File attachments in the input box (use `drop_target` primitive alongside)
- Voice input (separate primitive)

---

## 6. State Management

### Where messages live

**App-owned `user_state`**, serialized as JSON via the existing state protocol. The app is authoritative. Plexi caches the rendered layout but never owns the list.

```python
state["messages"] = [
    {"id": "m1", "role": "user", "parts": [...]},
    {"id": "m2", "role": "assistant", "parts": [...]},
]
```

Why not `persistent`? Persistent state is per-directory; a chat bound to a specific project fits, but ephemeral chats (e.g., a one-off ask in a throwaway pane) don't. Let the app decide by putting messages in whichever bucket fits its use case.

### Scroll position

**Plexi-owned**, keyed by `chat_id`. Survives redraws but not app restart. On app restart, Plexi restores scroll to bottom by default. If the app wants scroll-position persistence across restarts, it can pass a `scroll_y` field in the chat command (optional, v2).

### Streaming partial messages

The `streaming` field on the chat command tells Plexi "this message is still arriving." Plexi renders it with a pulsing cursor glyph at the tail. When the app emits `chat_stream_end`, Plexi clears the indicator.

During streaming, the app emits `chat_append_delta` commands. Plexi merges them into the addressed message. This is a separate draw command, not part of the `chat` command itself — delta streams must not force a full frame re-serialization.

### Focus behavior

When the user clicks the chat primitive's input box, that chat becomes the active input target for the pane. Keyboard events route there until the user clicks elsewhere or presses Escape. This is the same pattern used by `list` for keyboard-driven selection.

---

## 7. Streaming Protocol

Plexi's existing JSON pipe is newline-delimited JSON over stdin/stdout. The chat primitive uses it as-is — no WebSockets, no SSE, no new transport.

### Wire pattern during streaming

Frame N: app submits a `chat` command with `streaming: {message_id: "m3", done: false}` and an empty text part for m3.

Between frames: app emits incremental `chat_append_delta` commands as tokens arrive from its LLM. Each delta is one newline-delimited JSON object; Plexi applies it to its cached message state and repaints.

```json
{"type": "chat_append_delta", "chat_id": "main", "message_id": "m3",
 "part": {"type": "text", "text": "The concat"}}
{"type": "chat_append_delta", "chat_id": "main", "message_id": "m3",
 "part": {"type": "text", "text": " demuxer..."}}
```

Delta merge rule: if `part.type == "text"` and the target message's last part is also text, append `delta.text` to that part's `text` field. Otherwise, push a new part. This matches the Vercel `text-start` / `text-delta` / `text-end` semantics without requiring explicit start/end boundaries for the common case.

For non-text parts (`tool_call`, `code`, `image`), each delta emits a whole new part — no partial tool-call streaming in v1. Deferring partial tool-call streaming avoids the complexity of rendering half-built tool invocations, which every existing chat UI handles awkwardly anyway.

### End-of-stream

```json
{"type": "chat_stream_end", "chat_id": "main", "message_id": "m3"}
```

Plexi clears the pulsing cursor and freezes the message. If the app never sends an end and the chat command is re-submitted with `streaming: null`, Plexi clears it implicitly.

### Backpressure

Plexi batches deltas within a frame. If the app emits 50 deltas between repaints, Plexi applies all of them before the next draw. There is no per-delta redraw — deltas are merged into state and the next natural repaint reflects them.

### Rendering during streaming

Plexi must not re-parse markdown from scratch on every delta. The implementation should keep an incremental markdown state per message — either by rendering raw text with minimal inline parsing (bold/italic/code done char-by-char) or by re-parsing only the "active streaming" part's text on each repaint and caching the rendered layout of everything above it. Either is fine for v1; the cache-above-active approach is what T3 Chat does with the Marked lexer and is the right target if rendering becomes a bottleneck.

---

## 8. Relation to Existing Specs

### 8.1 `intelligence-protocol.md` (deferred)

The chat primitive is the **right place to revisit the intelligence-proxy question**, but not in v1. The recommendation above (Section 3) is: ship v1 as pure rendering, and only in v2 — after two apps are using it and we have real usage data — decide whether to add an opt-in `provider` field that routes through Plexi. When that happens, PR #108's `LlmWorker` pattern becomes the foundation (not the deferred intelligence-protocol spec's full tier-based design, which was heavier than needed).

### 8.2 `agent-mode.md` + PR #108

Agent mode is Plexi's *own* feature, not a third-party app, but it is a natural first consumer of the chat primitive. Today agent mode has custom rendering code in `agent_ui::render_agent_mode`. Once v1 of the chat primitive lands, agent mode should be migrated to use it — that's the validation test for the primitive. If the primitive can't host agent mode cleanly, it's not done.

PR #108's `LlmWorker` keeps running as a *Plexi-internal* backend: when agent mode uses the chat primitive, it's still the worker making the API call, not the chat primitive itself. This preserves the v1 "pure rendering" boundary cleanly.

### 8.3 `agent-replay-testing.md`

Agent replay captures structured traces of agent runs — prompts, tool calls, responses. The chat primitive is the natural replay viewer. When the replay spec eventually ships its UI, it should draw a chat primitive populated with the recorded messages, with extra affordances: diff-highlighting between branches, per-message cost/latency chrome, annotation overlays. The `chat_branch` event and the v2 reactions/annotations feature listed in Section 5 are direct dependencies of the replay UI — this is why they're in v2 scope, not deferred.

### 8.4 `core-advanced-ui-sdk.md`

The chat primitive is the highest-level UI primitive yet proposed and sets a precedent. It implies that "draw primitive" in Plexi doesn't have to mean "one rect" — it can mean "a stateful interactive widget with its own input handling and scroll and event vocabulary." That precedent was already set by `list`; chat just extends the ladder further. Future primitives (data tables, graph viewers, form builders) can follow the same shape.

### 8.5 Claude Code as backend (recent research)

A recent research thread asked whether Plexi's agent mode could use Claude Code as its backend. The chat primitive is orthogonal to that question — whatever agent mode chooses as its backend still renders through the chat primitive. If Claude Code becomes the backend, the chat primitive just receives deltas from Claude Code's session stream instead of from a direct Anthropic API call.

---

## 9. Open Questions

1. **How opinionated should the "bubbles" style be?** T3 Chat, ChatGPT, Claude, Cursor, Cline — all have visually distinct chat styling. If Plexi's default looks like any of them, apps will inherit that visual identity. The safer default is the `lines` style (terminal-native, minimal chrome), with `bubbles` as opt-in. Decide before v1 ships.

2. **Markdown subset.** Full CommonMark is heavy. Minimum useful: bold, italic, inline code, code blocks, links, lists. Tables? Images inline? Math? Defer all three to v2 unless a consumer explicitly needs them.

3. **Syntax highlighting dependency.** Every Rust syntax highlighter adds 1–3 MB to the binary. `syntect` is the quality option; `tree-sitter-highlight` is the performance option. Ship v1 with one-color-per-language (no actual highlighting) and pick a highlighter when the first code-heavy app needs it.

4. **Tool call rendering — collapsed or expanded by default?** Cline collapses. Vercel's default expands. The right answer depends on tool-call density. Default to collapsed; let the app pass `expand_tool_calls=true` if it wants otherwise.

5. **Should the primitive own the input box, or should the app draw its own?** Pro-own: less boilerplate, consistent UX. Con-own: apps with unusual input needs (slash commands, autocomplete, inline attachments) have to work around it. **Recommendation:** own the input box in v1, but support a `input_mode="external"` escape hatch where the primitive renders only the message list and the app draws its own input with standard `rect` + `text` commands. Worth including in v1 because agent mode will need the escape hatch.

6. **State-bucket guidance.** Messages in `user_state` (ephemeral, per-pane)? `persistent` (per-directory)? A dedicated chat log file? Give app authors a one-paragraph heuristic in the docs so every app doesn't re-solve it.

7. **When does v2's `provider:` field ship, and what does it look like?** The architectural debate reopens the moment this field is added. v2 should either (a) route through an OpenRouter-compatible endpoint (one key, provider pinned per model family to preserve caching) or (b) generalize PR #108's `LlmWorker` into a first-class Plexi subsystem. The decision framework: are apps asking for routing, or just for the UI? Measure before deciding.

---

## Sources

- [Grokipedia — T3 Chat](https://grokipedia.com/page/T3_Chat)
- [Theo Browne — T3 Chat redesign LinkedIn post](https://www.linkedin.com/posts/t3gg_t3-chat-is-now-the-cheapest-fastest-and-activity-7307892576580161536-eYBv)
- [Vercel AI SDK — Introduction](https://ai-sdk.dev/docs/introduction)
- [Vercel AI SDK — Stream Protocols](https://ai-sdk.dev/docs/ai-sdk-ui/stream-protocol)
- [Vercel AI SDK 5 announcement](https://vercel.com/blog/ai-sdk-5)
- [Vercel AI SDK v5 Internals — Part 4](https://dev.to/yigit-konur/vercel-ai-sdk-v5-internals-part-4-decoupling-client-server-state-management-and-message-1lb1)
- [Multi-Step & Generative UI — Vercel Academy](https://vercel.com/academy/ai-sdk/multi-step-and-generative-ui)
- [vercel/ai on GitHub](https://github.com/vercel/ai)
- [OpenRouter](https://openrouter.ai/)
- [OpenRouter Pricing](https://openrouter.ai/pricing)
- [OpenRouter — Prompt Caching docs](https://openrouter.ai/docs/guides/best-practices/prompt-caching)
- [OpenRouter — Provider Routing docs](https://openrouter.ai/docs/guides/routing/provider-selection)
- [OpenRouterTeam/ai-sdk-provider — Anthropic caching issue](https://github.com/OpenRouterTeam/ai-sdk-provider/issues/35)
- [OpenRouter Review 2025](https://skywork.ai/blog/openrouter-review-2025-unified-ai-model-api-pricing-privacy/)
- [TGlide/thom-chat](https://github.com/TGlide/thom-chat)
- [Hairetsu/NOT-T3-Chat](https://github.com/Hairetsu/NOT-T3-Chat)
- [shaltielshmid/NotT3Chat](https://github.com/shaltielshmid/NotT3Chat)
- [Cline — GitHub](https://github.com/cline/cline)
- [Cline docs — Chat Completions](https://docs.cline.bot/api/chat-completions)
- [Continue.dev CLI Tool architecture](https://deepwiki.com/continuedev/continue/10-cli-tool)
- [aider.chat](https://aider.chat/)

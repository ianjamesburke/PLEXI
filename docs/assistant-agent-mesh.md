# Assistant Agent Mesh

Status: active.
Stint: none yet — filed after the stable v1 cut lands.
Parent: [`assistant-host-app.md`](assistant-host-app.md).
Authority plane: [`assistant-authority-model.md`](assistant-authority-model.md).
Last updated: 2026-08-01.

This document defines the destination shape of the Plexi Assistant as a **mesh of head agents** rather than a single chat surface: one head per context root plus one global head, a shared central record they all write to and hold pointers into, a typed question channel between them, and an addressing scheme that makes every head individually testable.

It owns exactly one plane: **how multiple Assistant instances coexist, remember, and talk** — and, in §7, the one third-party surface that plane must terminate cleanly: MCP Apps, where an outside server hands a head a UI. It does not restate the Assistant's product surface (agents, skills, slash commands, settings, UI — [`assistant-host-app.md`](assistant-host-app.md)), the authority plane (threat model, reference monitor, grant binding — [`assistant-authority-model.md`](assistant-authority-model.md)), or host-owned workflow execution ([`agent-run-orchestration.md`](agent-run-orchestration.md)). Where this design needs a fact from those documents it cites them.

Its sibling is [`decision-trust-plane.md`](decision-trust-plane.md), which owns who resolves a judgment call when several answers are defensible: typed decision records that rise from worker to head to human, and per-category trust folded from their outcomes. It depends on §3's drain for storage and shares §6's escalation schema as its human-facing end. It changes nothing about authority, and neither does this document.

## Mandate

**This PRM may specify foundational changes to existing surfaces** — CLI namespaces, the Assistant app, the chat interface — and is not confined to additions that sit politely beside what is there.

The condition on that grant is the repo's existing standard, restated here because it is what makes the grant safe: every such change **lands complete and supersedes the old implementation outright.** No backwards-compatibility shims, no legacy re-exports, no dual paths kept alive "until callers migrate." Migrating every consumer is part of the change, not follow-up work, and a surface is not superseded until nothing reads the old one. Where this document rules that something is replaced, the appendix names what has to move.

## Sequencing

This is post-v1 work. [`2026-08-01-v1-cut.md`](2026-08-01-v1-cut.md) rules the Assistant, orchestration, browser, marketplace, native WASM distribution, and media editing out of the stable v1 sprint, and lists `agent-run-orchestration` under Post-v1. `NORTH_STAR.md` places the Assistant-as-workspace-operator in Phase 3 — Intelligence. Nothing in this document may be pulled forward into the v1 acceptance set.

Two hard prerequisites are not matters of scheduling, and both block work rather than merely preceding it. One is a sandboxed webview pane, without which §7 cannot begin at all; it is stated in full there. The other is root uniqueness.

A head agent addressed by its context root only exists if roots are unique. [`context-root-uniqueness-and-rollup.md`](context-root-uniqueness-and-rollup.md) records that duplicate roots are not merely possible today but guaranteed by two creation paths, and that two contexts sharing a root already collide on context-scoped app state. Per-context heads inherit that collision exactly — two heads at one root are two agents that believe they own the same working memory. The uniqueness ruling in that brief is a blocking dependency for this one.

## 1. What exists today

The Assistant is a builtin host app instantiated per pane. `AssistantApp::new` takes a `workspace_root`, a broker, a config dir, and a `context_id`, and `src/assistant/store.rs` already keys conversation state per host context: `state.toml` carries `[contexts.<context_id>]` tables holding each context's active conversation, session name, agent, and effort, while transcripts under `conversations/<id>.jsonl` are shared workspace-wide and claimed by the first context to activate them.

So per-context *conversation state* exists. Per-context *agents* do not. The agent is a property of the pane: two Assistant panes in one context are two independent agents with no relationship, and a context with no Assistant pane has no agent at all. No host-level Assistant outlives its pane.

**No CLI surface addresses an Assistant at all, and building one is a mandate of this document rather than a gap it notes.** `plexi ai` is configuration and diagnostics, `plexi agent` manages three unrelated things none of which is an Assistant, `plexi routine` schedules shell commands, and `plexi notify` raises notifications. Nothing sends a message to an Assistant and nothing names one. Under the fourth commandment that is not a missing convenience — an Assistant that the CLI cannot address does not exist for an agent, which makes it untestable in isolation (§5) and unreachable from a routine (§5). The namespace ruling, and what it supersedes, is in the appendix.

The pieces this design builds on are already load-bearing:

- **Tool scoping.** `ToolDispatcher::from_registry` snapshots the global registry through `snapshot_for_caller`, which compares the caller's workspace root against each registered pane's `workspace_root` component-by-component and excludes everything outside it. Cross-workspace tools are not merely unauthorized, they are absent from the model's tool list. `ToolDispatcher::add_host_tools` with a `HostToolHandler` is the existing seam for caller-local tools that resolve in-process and never reach a pane; `ToolCallHooks::before_call` is the existing per-call gate.
- **A conflict rule worth reusing.** When two panes in one workspace expose the same tool name, `snapshot_for_caller` withholds *both* and logs the conflicting pane ids rather than picking a winner, so callers get `tool_not_found` — the correct fail-visible signal. That principle governs ambiguous question routing in §4.
- **An append-only record.** `EventLog` and the `HostEvent` enum in `src/host/event_log.rs` write newline-delimited JSON to a global `events.jsonl` and, inside a workspace, a workspace-local one. The host stamps `source` before enqueueing; apps cannot forge provenance.
- **A cost record.** `src/plexi_ai/ledger.rs` appends one row per brokered call with app and model attribution, and `check_budget` gates every call against per-app and global daily caps.
- **Root-scoped schedules.** `RoutinesConfig::load_from_root` in `src/host/scheduler.rs` loads routines per context root, and `plexi routine` schedules them; today a routine's effect is a pane spawn.
- **Scoped user-facing messages.** `NotifyScope` in `src/protocol/commands.rs` is `Window | Context | Global`, defaulting to `Context`, resolved host-side from app policy.
- **Two backends worth distinguishing.** `LiveAiBroker` drives real models; `HostHarness::add_assistant_pane` in `src/testing/mod.rs` builds an Assistant over an inert broker whose `dispatch` returns an error and logs, which is what makes plumbing testable with no model in the loop. `LocalOpenAiBackend` (backend name `"local"`) speaks OpenAI-compatible SSE against a configurable `base_url` with optional auth, which is what makes a live local model reachable from a test.

## 2. Head agents

**A head agent belongs to a context, not to a pane.** Every context root has exactly one head. The host has exactly one global head that belongs to no context. A head exists whether or not anything is displaying it; an Assistant pane is a *view* onto its context's head, and closing the view does not end the agent.

This mirrors the scoping distinction the product already draws in two places: apps are scaffolded and installed either into the global registry or into a workspace (`plexi app init --global` versus the nearest workspace), and notifications are `Window | Context | Global`. A head is the same distinction applied to agency.

**Addressing.** A head's durable address is its context root path; its runtime handle is the `context_id`. The global head's address is the literal `global`. Volatile ids never appear in a written artifact — a routine, an agent definition, or a drain record names the root, and the host resolves it. This is why root uniqueness is a prerequisite rather than a nicety: the address must identify one agent.

Two facts from the appendix bound this section and are worth carrying here: a head's identity has to be one thing threaded through broker attribution, bus subscription, and the audit actor at once (H4, S8, A8 — three surfaces that each collapse every head into the same constant string), and the host can currently hold only one headless Assistant machine-wide (A5). Neither is a detail of §2; each one alone makes it unbuildable.

**What a head is for.** The global head answers about the machine and routes across contexts. A context head answers about its own root — its panes, its apps, its files, its work — and is the escalation target for anything scoped there. The division is relevance, not power.

**Authority is unchanged by this design.** [`assistant-authority-model.md`](assistant-authority-model.md) rules that a context root is a relevance boundary and not a security boundary, and that the host reference monitor is the only trusted authorizer. The mesh adds heads; it does not add an isolation claim, and no part of it may be cited as one. Two facts follow and must be stated in the product surface rather than assumed away:

- Tool visibility is scoped by **workspace** root, not context root (`snapshot_for_caller`). Contexts sharing a workspace see each other's tools. That is the existing contract, and per-context heads do not narrow it.
- A head holds no standing grant. Every action it proposes is authorized per-call at the reference monitor exactly as a single Assistant's is today.

**Configuration is explicit.** Which contexts get a head, each head's model and effort, the information domains it declares in its capability card (§4), its working-memory budget (§3), and its unprompted-message budget (§5) are declared fields. Required fields have no default and a missing one is an error naming the field and the file — never a silent fallback to a global setting.

## 3. Memory

### The ruling

**Heads keep no unbounded private log.** All assistant activity flows to one central drain. Each head holds a small, hard-bounded working memory whose evicted entries are replaced by pointers into that drain, and reads the drain back through a typed tool.

### Why not per-agent logs

The alternative — each head owns its own append-only history — was evaluated and rejected on three grounds. It multiplies an unbounded growth problem by the number of contexts instead of solving it. It makes "what did the fleet actually do" a join across N files that nothing enforces and nothing can audit, against `NORTH_STAR.md`'s exclusion of two sources of truth for state and the authority model's ranking of audit-trail integrity as a protected asset. And it defeats §4: routing a question to the head that owns a fact requires the asker's ability to find that fact to be independent of which head happens to hold it.

Embedded-vector memory was rejected on the first commandment. Memory that cannot be opened and read as text in a hundred years is not memory this product is allowed to ship.

### The drain

The drain is the existing append-only host event log, extended with assistant record variants — not a new log. One record class, one file family, one reader.

Three changes are required before it can carry memory, and each closes a real defect rather than adding a feature:

**Records need stable identifiers.** No `HostEvent` variant carries a record identifier. Every variant is stamped with a `timestamp`, and the `id` fields that do appear are domain ids belonging to the thing described — a notification's id, a run's id — not an address for the record itself. A pointer needs an address, and a timestamp is not one. Every drain record gets a durable, monotonic id assigned at write.

**The drain must be lossless for assistant records.** `EventLog`'s writer is a bounded channel with an explicit drop-on-full policy: at capacity the event is discarded and `dropped_count` is incremented, with no retry and no blocking. That is a defensible trade for UI telemetry and an indefensible one for the only record of what an agent knows — a dropped record is a pointer that resolves to nothing. Assistant records take a backpressured path; a write that cannot complete is an error surfaced to the head, never a silent discard.

Three further conditions come out of the substrate audit and are stated with the rest in the appendix (S1–S6), because each one independently makes a pointer unresolvable: a drain write issued from outside the host process currently succeeds while writing nothing, shutdown discards what is still queued, and the record format has no version or forward-compatible read. **Ids are necessary and not sufficient — the drain is memory only once a write that reports success has actually landed and a newer build's records are still readable by an older one.**

**Reads go through a typed surface.** A head recalls by tool call, never by opening a `~/.plexi*` path — the standing rule in `AGENTS.md` that app and host state is reached through SDK tools or the CLI. A recall is itself a drain record, so the audit trail records what an agent looked at and not only what it did.

**Read scope.** A context head recalls its own records and the records its context owns. The global head recalls across contexts. Nothing else widens: recall is subject to the reference monitor like any other tool call.

### Working memory

A head's working memory is bounded by a declared budget with no default. Eviction is mechanical, not editorial: an evicted entry is replaced by a pointer record — the drain record id plus a one-line descriptor sufficient to decide whether to recall it. A head that has forgotten something knows it has forgotten it and knows where to look. Compaction never rewrites history; the drain is append-only and the working memory is the only mutable surface.

### Relationship to what already persists

Conversation transcripts under `<workspace>/<channel>/assistant/conversations/` stay where they are and keep their format: they are the user's readable record of a conversation and an open-format artifact in their own right. The drain is the *cross-agent* record — what a head did, learned, asked, answered, and escalated. The cost ledger stays the cost ledger. No file gains a second writer, and no fact is written to two of them.

## 4. `ask_question`

A head that needs information another head or the human likely holds calls one tool. The tool decides the target.

### The tool

`ask_question` is a caller-local host tool registered through `ToolDispatcher::add_host_tools` and resolved by a `HostToolHandler` in-process. It is not a pane tool, because the human is a valid target and the human is not a pane.

### The registry is derived, never maintained by hand

**Every agent publishes a capability card at registration; the host aggregates the cards into the routing registry.** There is no file anyone edits to say who knows what.

A hand-maintained list is not a merely inelegant option, it is a known failure class in this repo. The babysitter ledger at `.agents/skills/babysitter/LEARNINGS.md` carries an explicit promotion rule: when repeated instruction fails to enforce something it is not a wording problem and the fix belongs in the host. Its L001 entry records six real violations of a written prohibition in a single night across three workers — one of them a fresh pane whose brief led with the rule — and the resolution was a host-enforced lock, not another rewording. A registry file that stays accurate only if every agent author remembers to update it is that same losing bet. Derivation is the enforcement.

**A card has a declared half and an enumerated half, and an agent cannot forge the second.**

- *Declared:* the information domains this head owns. Only a person can assert that a head is the authority on a project, so this is authored configuration on the head itself, carried in the card at publish time.
- *Enumerated:* the tools the head can actually reach, filled in by the host from the live registry — `ToolDispatcher::all_tools` for the head's own snapshot and `apps_for_workspace`, which already groups registered tools by the app that exposed them for `/apps`. A head does not get to claim a tool list. The list is what the host observes, the same way `EventLog` stamps `source` before enqueueing so apps cannot forge provenance.

**Card schema borrows the A2A Agent Card's shape; the mesh does not adopt the A2A wire protocol.** The useful part of Google's Agent Card is the shape: one JSON capability manifest per agent covering identity, provider, capabilities, discrete named skills, interface, and security schemes, resolved by lookup rather than by convention ([spec](https://a2a-protocol.org/latest/specification/)). Taking that shape means Plexi's cards are already legible to an external bridge if one is ever wanted, at zero cost today.

Two things are deliberately not taken. A2A's own discovery guidance offers a well-known HTTP URI, curated registries, and direct configuration ([agent discovery](https://a2a-protocol.org/latest/topics/agent-discovery/)); the mesh takes the aggregated-registry model and explicitly rejects the direct-configuration model, which is the hand-maintained file this section exists to eliminate. And the transport stays Plexi's. Cards publish through the existing registration path — the same shape `ExposeTools` already uses to reach `tool_dispatch::register` — and structured cross-agent data that is not a host tool call rides the event bus (`DeclareEventStreams` / `EmitEvent` / `SubscribeAppEvents`), per the one-comms-model rule in the root `AGENTS.md`. A2A or ACP compatibility, if it ever lands, is an external bridge at the edge and is out of scope here. Nothing internal speaks a foreign protocol to reach the agent next to it.

**MCP stays the tool surface and is enumerated too.** MCP remains how external tools and UIs reach the mesh (§7); it does not become the inter-head channel. A head's MCP servers and their tools appear in its card by enumeration from what is actually registered, never from a maintained list — the same rule as every other entry, for the same reason.

### Routing

The handler matches the question against the declared domains in the aggregated cards and resolves exactly one target:

- One head claims it → route to that head.
- No head claims it → route to the human.
- More than one head claims it → route to the human, naming the claimants. This is the `snapshot_for_caller` conflict rule applied to a second surface: when ownership is ambiguous the correct behavior is to fail visibly to a person, never to pick a winner silently. The *principle* transfers; that rule's opaque `tool_not_found` result does not, because an asker must learn why it was routed to a human (appendix, H-notes).

**Human routing uses the notification surface** with its existing scope semantics, not a new channel.

**An answer is information, never authority.** `ask_question` returns data. It cannot make another head act, and a head cannot use it to reach a tool its own reference-monitor check would deny. Asking a head that can do something is not a way to do that thing — that is the confused-deputy path the authority model exists to close, and the mesh must not reopen it.

**Bounds.** Every ask carries a deadline. A question that would close a cycle — A asking B while B is blocked on A — is refused at the handler with the cycle named. Ask, answer, refusal, and timeout are each drain records.

## 5. Individually testable, and what ships because of it

**Every head is addressable by direct message.** The `plexi assistant` namespace addresses a head by root or `--global` and delivers a message to it, returning its reply — and appending the exchange to that head's conversation (§8). This is the fourth commandment applied to agents — if a head is not reachable from the CLI it does not exist for an agent — and it is simultaneously the test harness: a head that can be messaged in isolation can be tested in isolation, with no pane, no window, and no sibling heads.

**Two test tiers, both required.**

*Plumbing, no model.* The inert-broker harness (`HostHarness::add_assistant_pane`) proves head creation and lifetime, addressing, drain writes and pointer resolution, question routing including the ambiguous and cyclic cases, and scope enforcement — all without a model, all deterministic, all in the default suite.

*Behavior, live model.* A head's actual conduct — that it asks rather than guesses, escalates in the required shape, speaks unprompted only when the filter says so — cannot be proven by an inert broker. Those tests run against a live model through `LocalOpenAiBackend` with `backend = "local"` pointed at a local Meridian proxy (stint 0579). They assert on **observable structure**: which record kinds appear in the drain, which target an ask resolved to, whether an escalation carries every required field. They never assert on model prose, because prose is not a contract.

This gate has a hard precondition that does not hold today: reasoning effort is silently dropped on the local backend, so a head's declared effort never reaches the model the gate runs against (appendix, H10). A behavioral assertion over a model that did not receive the head's own configuration proves nothing about the head.

**The behavioral suite is a gate, not a skip.** It is a named gate requiring a reachable local backend, and it **fails** when that backend is unconfigured rather than passing quietly. [`2026-08-01-v1-cut.md`](2026-08-01-v1-cut.md) makes the same call for the release gate: a gate carrying a known skip is not a gate. A local model is cheap enough that "the model was not available" is a broken environment, not an exemption.

**What compounds on top.** Three capabilities become reachable once a head is addressable, and each extends an existing primitive rather than adding infrastructure:

- **Routines that address a head.** `RoutinesConfig::load_from_root` already scopes schedules to a context root; today a routine's effect is a pane spawn. A routine gains a head as a target, so "ask this head this, on this schedule" needs no pane and no terminal. Work a routine kicks off that outlives the tick is a task in the MCP Tasks shape (§7), not a bespoke job record.
- **Information cleanup as a routine.** Working-memory compaction (§3) is a scheduled pass a head runs against itself, using the same eviction rule as ordinary eviction. One compaction mechanism, invoked two ways — never a background daemon with its own policy.
- **Unprompted messages.** A head may speak first, through the notification surface at its declared scope. Every unprompted message is a drain record and counts against a declared per-head budget with no default. The escalation contract in §6 governs *what* is worth saying; this governs how it arrives and how often.

## 6. Migration from the personal chief-of-staff prototype

Ian's `me` system prompt is a working prototype of what a head agent eventually runs: it has been operating a fleet, distilling status, and escalating to a human under written rules for months. The behaviors below graduate from prose instructions to product mechanisms. The prompt's own text is not the spec — it is evidence that these behaviors are load-bearing.

**Escalation as a typed record, not a convention.** The prototype's escalation contract requires every item raised to a human to state what the thing is in plain language, what each option means, the trade-off, a recommendation stated as a call, and the time it will cost. In prose that rule is violated regularly and only caught by the human it failed. As a schema it cannot be: an escalation missing a field is not raiseable. This is the highest-value item in the migration, and it is the reason §4 routes ambiguity to a human through a typed surface rather than a free-text message.

**Distillation as the default output shape.** The prototype's rule that a status report contains only what needs the human — and that a healthy fleet is one line — becomes the head's filter on unprompted messages and the default shape of any summary it produces. Silence is a valid and common report.

**Liveness must be observed, never claimed.** The prototype learned the hard way that a self-reported status slot reading `running` is a claim, not evidence, and can sit green over a dead agent for hours. A head reporting another agent's state cites an observation — a drain record — never a self-report. This is what §3's lossless-drain requirement buys.

**Dispatch and end-state ownership.** A head hands work to workers and owns their reaching a real end state rather than a hopeful one. The execution machinery for this is [`agent-run-orchestration.md`](agent-run-orchestration.md)'s durable run record; the mesh contributes the head that owns the runs and the drain the run records land in. Not restated here.

**One board, rendered.** The prototype keeps a live focus board and its hardest recurring failure is that board drifting from a second surface rendering the same items. A head's board is a projection of the drain plus open escalations. It is never an independently maintained file, and the mechanism that prevents drift is that there is nothing to drift from.

**What does not migrate.** The prototype's voice rules, ADHD-calibrated output mechanics, energy-curve scheduling, and personal file layout are one person's persona. They ship to nobody by default. They belong in the per-head authored configuration that §4's capability card already carries — and they are exactly what "grown, not universal" means in `NORTH_STAR.md`. A head with no user-authored persona is still a complete head.

## 7. MCP Apps as the third-party UI surface

### Why this plane belongs here

The MCP 2026-07-28 release restructured the protocol into a **stateless request/response core plus formally versioned extensions** — session handshakes and the `Mcp-Session-Id` header are gone, method and tool names travel in `Mcp-Method` and `Mcp-Name` headers, server-initiated streams are replaced by multi-round-trip requests, and list results became cacheable ([release notes](https://blog.modelcontextprotocol.io/posts/2026-07-28/)). Two of the formal extensions are exactly the shapes this design would otherwise have had to invent: **MCP Apps**, a UI surface a server can hand a host, and **`io.modelcontextprotocol/tasks`**, a contract for long-running work.

Adopting both means the mesh invents neither a third-party UI protocol nor a job protocol. That is the whole argument for this section: everything below is a decision *not* to build something.

### One centralized MCP-app pane type, host-owned

**A single Plexi pane type renders MCP Apps and implements the host side of the spec. Servers never get custom pane types.**

The pane renders `ui://` HTML resources — the spec reserves that scheme and pins the MIME type to `text/html;profile=mcp-app` — and speaks the postMessage JSON-RPC 2.0 bridge the spec defines, which deliberately needs no SDK on the guest side ([apps extension](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)). The boundary is the spec's own wire protocol and nothing else.

The alternative — a pane type per server, or per app — is N boundaries to audit, N places for authority to leak, and N implementations of a protocol that already has one definition. This repo has already made the same call once: `tool_dispatch` is a single global registry with one `dispatch_call` path rather than per-app plumbing, and that is why workspace scoping could be added in one place and be true everywhere.

**Visibility is enforced where visibility is already enforced.** Tools carry `_meta.ui.visibility`, whose values are `"model"` and `"app"`, defaulting to both; the spec requires that a host **must not** include a tool in the agent's tool list when its visibility omits `"model"`. In Plexi that is the dispatcher snapshot: an `"app"`-only tool is callable by the iframe across the bridge and absent from the head's `all_tools()`. The precedent is `retain_allowed`, which removes a tool from *both* model visibility and invocation — gating one without the other is precisely the defect this repo already closed, and the MCP rule is the same rule.

**Forwarded messages are addressed, never ambient.** `ui/message` and `ui/update-model-context` go to the head that owns the pane's context (§2) — not to "the Assistant." This is the concrete payoff of per-context heads on this surface: an MCP app running in one context cannot write model context into another context's head.

### Mapping the spec's chat-iframe flow onto panes

The spec assumes a chat host with an embedded iframe. Plexi is a tiling environment, and every flow maps onto a pane operation the host already performs:

| MCP Apps message | Plexi host behavior |
|---|---|
| `ui/initialize` | Handshake. Host returns `hostContext` — `theme` plus the `styles` variable set — and the display modes this pane can honor. |
| `ui/notifications/initialized` | Pane marked live. |
| `ui/request-display-mode` (`inline` / `fullscreen` / `pip`) | Pane placement: `inline` is the tiled pane, `fullscreen` is zoom, `pip` is the existing pip surface (`AppPane::pip_status`). |
| `ui/notifications/size-changed` | Pane sizing request. |
| `ui/resource-teardown` | Host notifies the view before closing the pane. |
| `tools/call` | `ToolDispatcher::dispatch_call`, gated by `_meta.ui.visibility` and by `ToolCallHooks::before_call` like every other call. |
| `ui/notifications/tool-input`, `-partial`, `tool-result`, `tool-cancelled` | Delivered from the dispatch the host is already running; no second execution path. |
| `ui/message` | A user-visible message into the owning head's conversation. |
| `ui/update-model-context` | Context for the owning head's future turns. |
| `ui/open-link` | Host-mediated, subject to the reference monitor. |
| `notifications/message` | A drain record (§3). |

**A request is a request.** `ui/request-display-mode` and `ui/notifications/size-changed` are proposals the host arbitrates against the current layout. A guest never commands the tiling. That is the authority model's position applied to geometry, and it is why these map to pane *operations* rather than to direct writes.

### Keyboard support for MCP-app panes

**A focused iframe already receives keyboard input.** Basic keyboard interaction inside an MCP app needs no protocol change and no Plexi work: an app that binds keys in its own HTML works the day the pane exists. What the spec cannot express is *declaration* — a server has no way to tell a host which shortcuts it offers, so the host can neither render them nor arbitrate a collision. Everything below closes the declaration gap and nothing below touches the input path.

**Decision: a Plexi vendor extension in the spec's own extensibility seam.** Shortcuts are declared in `_meta.ui` on the UI resource — the same namespace the spec already uses for `visibility` and `resourceUri` — each entry carrying a key combination, a short label, and a one-line description, with a runtime notification for shortcuts that change as the app changes mode. The extension is named under `io.plexi/*`, matching the reverse-DNS shape of the release's formalized identifiers (`io.modelcontextprotocol/tasks`), so it can be proposed upstream as a SEP later without a rename.

Degradation runs both ways, and that is the design rather than a caveat: a non-Plexi host ignores an unknown `_meta` key and loses nothing, and a server that declares no shortcuts still works mouse-only exactly as it does today. Plexi-compliant MCP UIs with *optional* keyboard support.

**Declared shortcuts render in the pane footer through the primitive that already exists.** `HintBar` and `HintGroup` in `src/ui/hints.rs` are the host's key→label footer — the Assistant's own composer renders its bindings through them. A declared MCP shortcut becomes a `HintGroup` entry and nothing else is built. That primitive's visual redesign is stint `0699`'s scope, filed on Ian's read that the current key→label footer is an ugly primitive, with a design deliverable rather than an implementation. This section consumes whatever `0699` produces: no second footer, no MCP-specific chrome.

**Key arbitration is fail-visible.** Host-reserved chords — pane navigation, fullscreen, the rest of the host keymap — are never forwarded to the iframe. The host consumes them first, so an MCP app can never capture the keys a user needs in order to leave it. Everything else forwards while the pane is focused. A declared shortcut colliding with a reserved chord is **refused at declaration, with the collision named**, never shadowed silently at runtime — silent shadowing produces a footer advertising a key that does nothing, which is worse than an absent shortcut. This is §4's routing principle on a third surface: when two claimants want one name, fail where a person can see it rather than picking a winner quietly.

### Observability is centralized by construction

Every iframe↔host message crosses one bridge. Instrumenting that bridge instruments every MCP app that will ever be installed — no per-server work, and more importantly no per-server *gap*. The records land in the same drain as everything else in §3, carrying the attribution `tool_dispatch` already logs on every invocation: caller app id and pane on one side, provider pane and tool name on the other.

This is what makes the centralized pane type an audit property rather than a tidiness preference. The root `AGENTS.md` rule that no capability ships without at least one `info`-level trace is satisfied once, at the bridge, instead of being re-litigated for each server someone installs.

### Theming: one story, one pipeline

MCP Apps delivers theming through `hostContext` at `ui/initialize`: a `theme` of `"light"` or `"dark"`, plus `styles.variables` — a standardized set of CSS custom properties spanning color (`--color-background-primary`, `--color-text-primary`, `--color-border-primary`), typography (`--font-sans`, `--font-weight-bold`), spacing (`--border-radius-md`), and shadow (`--shadow-lg`).

**Plexi's app theme tokens align to that variable set, and one resolver serves both Python apps and MCP apps.** The host's tokens already live in `src/ui/theme.rs` behind `preset_names`, `preset_colors`, and `text_on`, with a WCAG contrast gate over every preset. What is missing is delivery, not tokens: stint `0696` is fixing exactly this pipeline, and the defect it names is that the Python app launch payload in `src/host/wasm_python.rs` carries a literal `"theme": {}`, so apps ignore the active host theme entirely and fall back to SDK dark defaults.

That is the same defect MCP apps would hit, so it gets the same fix and not a second one. `0696` is v1 work; MCP-app theming attaches to whatever pipeline it lands and adds no parallel path. The WCAG gate extends to the emitted variable set — a theme that ships an illegible `--color-text-primary` to an MCP app is the same class of bug the host already refuses to ship to its own widgets.

### The enabling primitive this section depends on

**Plexi has no webview today, and every decision above is inert without one.** The host is egui; no webview crate appears in the dependency set. [`2026-08-01-v1-cut.md`](2026-08-01-v1-cut.md) places `browser-surface` post-v1 on exactly this ground — no browser pane exists, so there is no half-working stable surface to clean up. Rendering `ui://` HTML requires a sandboxed webview pane, and that primitive is a **dependency of this section, not a detail inside it**.

Two consequences, both stated rather than assumed:

- **Ownership of the webview primitive is an open decision.** [`browser-surface.md`](browser-surface.md) covers a native browser pane and plausibly owns the same primitive. It is owned by one document and consumed by the other; which way round is a ruling to take before either is stinted, not a thing to discover during implementation by building it twice.
- **This section does not partially land.** There is no degraded mode where an MCP app renders as text or as a summary. A half-rendered third-party UI misrepresents what the user is looking at, and a UI surface that lies is worse than an absent one.

### Long-running work uses the Tasks extension

`io.modelcontextprotocol/tasks` graduated from experimental core to a formal extension in the 2026-07-28 release, with poll-based `tasks/get` and `tasks/update` plus a subscription channel for listening.

**That is the shape for long-running work a head dispatches.** §5's routines and unprompted messages use it rather than a parallel invented mechanism, and [`agent-run-orchestration.md`](agent-run-orchestration.md)'s durable run record is the local implementation of the same contract rather than a competing one. A routine that addresses a head produces a task; the head observes it by poll or subscription; a completion is what licenses an unprompted message under §5's budget. One job model, MCP-shaped, so external tooling can observe Plexi's long-running work later without a translation layer being retrofitted.

## 8. Conversation surfaces

A head is one agent with more than one way to reach it. These three surfaces are what keep that from fragmenting into several agents that happen to share a name.

### One canonical conversation per head

**A head has exactly one conversation, and every surface reads and writes that same thread.** The pane, the CLI, and any future API are views onto one record, never onto per-surface threads that have to be reconciled afterwards. Named sessions — several threads a user can switch between — are at most a later layer built on top of this, and are not part of the destination state described here.

That is a supersession, not a description. Today a workspace holds many conversations with a per-context active pointer: `state.toml` carries `active_conversation` and `session_name` per context, transcripts accumulate as separate `conversations/<id>.jsonl` files, `ConversationHistory::context_id` claims an untagged conversation for the first context that activates it, and `/new` and `/resume` move the pointer between them. Under this ruling the head *is* the thread's identity. The pointer, the claiming rule, and the surfaces that switch threads all collapse into it, and the appendix carries what that costs.

**A message sent to a head from the CLI appends to that head's conversation.** The command sends, prints the reply, and returns; opening the Assistant app on that head afterwards shows the exchange as ordinary user and assistant rows, in order, indistinguishable from one typed into the pane. Not a mirror and not a second session — the same thread, seen from another place.

The store already supports this: `AssistantStore` writes one `Turn` per line to `conversations/<id>.jsonl`, and the pane reads that file. What blocks it is ownership, not format — `AssistantStore` is a field of `AssistantApp`, so the transcript is only reachable while a pane holds it. This surface is therefore a consequence of §2 rather than an independent feature, and it does not work until the head owns the store.

**One writer, always the host.** The CLI never touches the transcript file. It delivers a message to the head over the same socket every other pane command uses, and the head writes. This is not a stylistic preference: `store.rs` rewrites the whole transcript on each save, because turns can finish out of submission order and disk order has to mirror memory order by rewrite. A second process appending to that file would silently drop turns on the next rewrite. It is also the standing rule that host state is reached through the CLI and never through a `~/.plexi*` path.

### The picker

**Opening the Assistant app lands directly in this context's head conversation.** No chooser, no intermediate screen, no decision asked before the user has expressed one. The common case is that a person in a context wants that context's head, and the app opens on it.

**A keybind opens the picker** — a palette of every head reachable from where you are standing, this context's and the global one, each row chipped to mark which kind it is. Enter turns the pane into a chat with that head. The picker is one keypress away and is never what greets you.

This is the Cmd+P palette, not a lookalike built next to it: the same `draw_command_palette` surface and `PaletteEntry` rows in `src/overlays/command_palette.rs`, with the same matching and ranking (`sort_palette_entries`), and the kind chip using the row-metadata affordance `PaletteEntry::Context` already carries. A second palette implementation with its own key handling and its own ranking is exactly the drift this repo keeps paying for.

The reachability model is already right there and worth reusing deliberately: `palette_agent_rows` collects agent rows from every window of every context, on the stated principle that the fleet is addressable from wherever you happen to be standing. Heads adopt that principle.

**Heads are a new row kind, and they stay visibly distinct from the existing agent rows.** Today's `AgentRow` is an agent-*bearing pane* — keyed by `window_id` and `pane_id`, populated from hook-reported `PaneAgentState` with `AgentState::Working | Blocked | Idle`, describing an external coding agent running in a terminal. A head has no pane, is addressed by context root, and carries different authority. Merging the two into one undifferentiated "agent" row would make the list shorter and the model wrong.


### Inter-agent receipts in the chat

A head keeps working while nobody is looking at it — most importantly, it answers `ask_question` from other heads (§4).

**That traffic renders inline in the thread, in chronological order, interleaved with ordinary messages.** Each receipt is a compact expandable one-liner: direction in or out, the counterpart head, a one-line summary, a timestamp. Not a sidebar, not a separate log, not a collapsed block at the top of the session — the conversation is the whole record of what this head did, and scrolling back through it shows the peer traffic sitting exactly where it happened relative to what the user said.

Same visual class as the tool-call receipts already in the transcript, for the same reason: it is something the agent did that the user did not ask for and needs to be able to see.

What changes in the chat interface, concretely: `TurnRole` gains a variant for peer traffic. The precedent is `TurnRole::Event`, which already exists for an app event delivered through a granted subscription — a row that is neither the user nor the model and still belongs in the transcript. Receipts reuse the tool row's existing affordances rather than inventing chrome: `Turn::status` distinguishes answered from failed or refused, and the caret dropdown that already shows a tool call's compact input summary and `detail` payload shows the full exchange.

One precondition, because it is structural rather than incidental: a transcript row carries no identity and no author today, and rows are addressed by position (appendix, A2). A receipt has to name its counterpart and join back to a drain record, so **`Turn` gains a stable id and an author before any of this is implementable.**

**A receipt is the §3 drain rendered into the conversation view.** The drain is the record; the receipt is a projection of it with no independent persistence and no second write path. This is the discipline §6 states for the focus board, applied to the transcript: one source, rendered, never a second file that can disagree with the first.

## Appendix: What this tramples

Everything below is a supersession the Mandate authorizes, with what has to move. It is a survey of the current code, not a plan — sequencing lives in stint tasks.

### The CLI namespaces as they actually are

Per subcommand: what it does, whether it is load-bearing, and who calls it. **Nothing in these three namespaces is dead.** That is the finding — consolidation cannot be justified as cleanup, only as naming.

**`plexi ai`** — implemented in `src/cli/ai.rs`.

| Verb | Does | Status | Callers |
|---|---|---|---|
| `doctor` | Read-only AI integration health check | Load-bearing | Documented; the diagnostic path `onboard` reuses |
| `onboard` | Guides first-run setup, then names the next app to install | **Orphaned** | One reference, in `README.md`. Nothing else invokes it |
| `setup` | Interactive Ollama configuration wizard | Load-bearing **and the sharpest hazard in the CLI** | Documented |

The namespace is configuration and diagnostics end to end. **No verb sends a message to anything.** `ai` is the most conversational noun in the CLI and nothing under it is conversational.

**`plexi ai setup` is destructive to this design specifically.** It rewrites config through `strip_ai_section`, which drops every `[ai]` and `[ai.*]` line — so one run deletes `[ai.local]`, both budget caps, and every comment in that region, then appends fresh sections. The `[ai.local]` block is the backend §5's behavioral gate runs against, and the caps are what H3 above depends on. A user running the documented setup wizard silently disarms the gate and the budget ceiling in the same command. **Obligation: config edits go through a real TOML editor that preserves unrelated keys and comments, not a section-strip-and-append.**

**`plexi agent`** — implemented in `src/cli/agent.rs`. Three unrelated referents share one noun.

| Verb | Referent | Does | Status | Callers |
|---|---|---|---|---|
| `init` | An agent *app* | Pure alias for `app_init(name, "python_agent")` — app scaffolding, nothing agent-specific beyond the template | Thin but real | Survives as the replacement for the removed `app init --agent`; a public SKILL.md cites it |
| `add` / `update` / `list` | An agent *definition* | Copies `AGENT.md` into `<workspace>/<channel>/agents/<name>/` with empty `memory/` and `logs/` | **Demo-grade — nothing in the host reads the result** | `plexi agent list` only |
| `report` | An external *process* | Sends `set_agent_state` over the pane socket → `AppRequest::SetAgentState` → Cmd+P rows and activity pips | **Load-bearing, and a machine ABI** | Three *externally installed* consumers: hook entries written into `~/.claude/settings.json`, the Codex hooks file, and the pi extension |
| `status` / `hook` | An external *process* | Human and skill entry points onto `report`'s mechanism | Load-bearing | Generated CLI docs; `assets/hooks/`; a currently-disabled dispatch skill |

**Two rows deserve correction and emphasis.**

`add`/`update`/`list` do not reach the Assistant, contrary to what the directory layout suggests. Two registries share `agents/` and disagree about what an agent is: `AgentRegistry::load` requires a `settings.toml` in the directory and **skips AGENT.md-only directories on purpose**, with a comment in `src/agent/mod.rs` naming `plexi agent add` as the producer of exactly the layout it skips; `agent_list` does the opposite, requiring `AGENT.md` and ignoring `settings.toml`. The result is that an added agent appears in `plexi agent list` and never in the Assistant, **with no error at either end**. The collision is documented as intentional, which makes it a design decision to revisit rather than a bug to fix quietly.

`report` is the opposite case — the most load-bearing verb in the namespace, and the one that cannot be renamed by a pull request. Its consumers are hook entries already written into users' `~/.claude/settings.json`, a Codex hooks file, and a pi extension. Those live outside the repository on machines the repository cannot reach, so **a rename breaks every installed hook silently**, on someone else's disk, at a time nobody is watching. Any change here is an ABI change and needs versioning, not a migration pass.

**"Agent" already means three incompatible things, and only one of them is in this namespace.** Besides `plexi agent` above and this document's heads, there is `[agents].low|medium|high` in config — coding-agent shell templates (`AgentsConfig`, `KNOWN_AGENTS`) consumed by `pane new --agent` and by the babysitter fleet's boot path. That third meaning is untouchable: **a consolidation that touches `[agents]` or the `--agent` flag breaks fleet boot.** Any naming ruling has to name which of the three it is moving.

**Three things that would let a namespace change break quietly**, all of which the Mandate's no-shim rule makes into obligations rather than footnotes:

- **The skill-surface gate reads only one file.** It checks `skills/plexi-cli/SKILL.md`, while live invocations also sit in `README.md`, the website drafts, `docs/pgap.md`, and the disabled dispatch skill — checked by nothing. Its coverage floors sit well below current coverage, so a namespace deletion is silently absorbed rather than failing the gate.
- **The website CLI reference regenerates only through self-committing recipes**, per the trap recorded in the root `AGENTS.md`. Documentation drift after a rename is the default outcome, not the unlucky one.
- **`plexi agent add` already prints a path it does not write** — it reports `.plexi/agents/` while writing the channel directory, and the published CLI docs repeat the wrong path. The surface is documented incorrectly today, before anything moves.

**Two seams are clean and should stay that way.** `NotifyScope` and `RoutineCmd` have zero coupling to the agent namespaces — which is exactly what §5 assumes when it puts heads behind routines and notifications. Neither needs to move for any of this.

**`plexi routine`** — `list`, `run`, `add`, `remove`, `enable`, `disable` over `routines.toml`, schedules validated against the same parser the scheduler fires on, so an accepted routine is guaranteed to fire. Load-bearing and internally coherent; the single noun means one thing. Its only limit for this design is that a routine's effect is a shell command in a spawned pane, which §5 extends without disturbing the rest.

### CLI consolidation — open ruling

**This is not settled.** Ian's question was whether the `ai` and `agent` functionality needs to be maintained or is just confusing. The inventory answers it per verb, and the answer is mixed: one machine ABI that must not move, one wizard that is actively harmful, one orphan, and one whole registry that nothing reads.

**Recommendation, stated as a call, verb by verb:**

- **Keep the `report` / `status` / `hook` mechanism, relocated but ABI-versioned.** The behavior is genuinely load-bearing and its consumers are installed on users' machines. Relocation is fine; a silent rename is not. Whatever it is called, the old invocation keeps answering under an explicit version contract until installed hooks have had a path to update — **and that is the one place in this document where an old surface may outlive its replacement, because the callers are not ours to migrate.**
- **Keep `doctor`. Fold `setup` into it behind a real TOML editor.** The wizard's function is wanted; its implementation deletes user configuration.
- **Delete `onboard`, or wire it.** A single `README.md` reference is not a live surface. Either it earns a caller or it goes.
- **Delete the `add` / `update` / `list` definition registry outright.** Nothing in the host reads it, and §4's capability cards supersede what it was reaching for. This is the one clean deletion available, and it is only clean *because* the audit showed the registry is unread — the earlier reading of this document, that `AgentRegistry` consumed these directories, was wrong.
- **Fold `init` back into app scaffolding.** It is an alias for `app_init` with a template argument and does not need a namespace.
- **Add `plexi assistant`** for heads: send a message, list reachable heads, show one, addressed by context root or `--global`. Rejected — folding heads into `plexi ai`, which is setup and diagnostics; rejected — taking `plexi agent`, which is where three meanings already collide.

**The cost is the external hooks, and it is the whole cost.** Everything else here is repo-local and moves in one change. The hook consumers live in `~/.claude/settings.json`, a Codex hooks file, and a pi extension on machines this repository cannot reach, which is why they get a version contract instead of a migration. Ian rules on the naming and on whether the `report` mechanism moves at all.

### What assumes a pane-owned agent

`AssistantApp` owns the entire agent. Its fields include `model`, `store`, `broker`, `agent_registry`, `skill_registry`, `settings_loader` and `settings`, `grant_store`, `audit`, every worker channel (`outcome_tx` / `outcome_rx`, `delta_rx`, `flow_tx` / `flow_rx`), and `pending_reply` — the channel a worker blocks on while a permission sheet is answered. It is constructed in `src/pane_ops/create.rs` and boxed into `AppRuntime::Builtin`, so its lifetime is exactly the pane's.

What has to be reworked, in dependency order:

1. **Split the head from the view.** Model, store, broker, registries, grants, audit, and the in-flight turn move to a host-owned head keyed by context. The pane keeps rendering, composer input, and pane-local UI state. `AssistantApp` becomes the view onto a head rather than the head itself.
2. **An in-flight turn currently dies with its pane**, because the worker channels are pane fields. A head that answers a CLI message (§8) or an `ask_question` (§4) with no pane open requires the turn to be owned by the head. **This is the largest single piece of work in the PRM**, and every other surface here is blocked behind it.
3. **`pending_reply` assumes a rendered permission sheet.** A blocked worker waits on a `SyncSender<PermissionReply>` that a pane-drawn modal answers. A headless head reaching an ask-tier tool has nobody to draw it. Destination: it escalates through the notification surface under §6's contract. Until that exists, a headless head is confined to what its standing posture already permits, and that confinement is an explicit config field, never an implied behavior.
4. **The multi-conversation model collapses into one thread per head.** This is the §8 ruling's cost, and it is larger than it looks. `state.toml` keys `[contexts.<context_id>]` — context-scoped, which is half right — but each of those tables carries an `active_conversation` pointer and a `session_name`, transcripts accumulate as separate `conversations/<id>.jsonl` files that are workspace-wide, and `ConversationHistory::context_id` claims an untagged conversation for whichever context activates it first. The slash commands that move the pointer (`/new`, `/resume`) are surfaces on that model. Under one canonical thread per head, the pointer, the claiming rule, and the thread-switching commands all go, and "unclaimed, visible to every context" stops being a reachable state. Existing multi-conversation transcripts on disk are the migration, and the no-shim rule means they are converted in the same change, not read through a compatibility path.
5. **Broker attribution is per-app, not per-head.** `AiBrokerRequest` carries `app_id`, and `ledger::check_budget` enforces per-app daily caps against it. Every head reporting as the Assistant shares one budget line and is indistinguishable in the ledger, so per-context spend is unknowable and one busy head can exhaust every other head's budget.

### Broker-layer hazards

The broker was built for one Assistant driving one workspace from one pane. Every assumption in that sentence is load-bearing somewhere in `src/plexi_ai/`, and the mesh breaks all three. Ranked by severity: each entry is what is true today, what it does under a mesh, and the obligation that follows.

**H1 — Tool-call ids are not unique across callers, and the pending-call map is global.** `dispatch_inner` registers every in-flight call in one process-wide `pending_calls` map keyed by `call_id`, while the broker mints that id as `format!("{}-{}", tc.id, i + 1)` — the model's own tool-call id plus an index within the turn, with nothing identifying the caller. Two heads dispatching concurrently can mint the same key; the second insert replaces the first head's `SyncSender`, so head A blocks until its timeout while head B is handed A's result. Local backends make this likelier, not rarer, because their ids are deterministic rather than random. **Obligation: `call_id` carries head identity and is unique by construction, and a resolution for an unknown or foreign key is an error rather than a silent replace.** This is the worst hazard in the layer — it is a cross-head data leak that presents as a timeout.

**H2 — The host-tool seam cannot carry `ask_question` as specified.** `add_host_tools` stores a single `host_handler` and overwrites it wholesale on every call, so there is one handler slot per dispatcher. Host-tool dispatch has no timeout of its own, `retain_allowed` filters only `self.tools` and never `host_tools`, and reply paths are pumped per-frame by a *viewing* pane — so a head with no pane open that calls `ask_question` parks its worker indefinitely. §4's deadline is not merely unimplemented on this seam, it is unimplementable. **Obligation: host tools gain their own timeout and gating, the handler becomes a keyed set rather than one slot, and servicing moves off the viewing pane's frame loop — this is the same `App::logic` discipline the root `AGENTS.md` already requires for every other external-client drain.**

**H3 — The budget gate is pre-flight, per-turn, and fails open.** `check_budget` runs before a turn against `today_spend`, which returns zero spend on any I/O or parse error by design, while the authoritative cost row is written seconds later from a detached thread after the generation cost is fetched. N heads waking on one routine tick all read the same pre-spend total and all pass, so the daily cap can be exceeded by N full turns before anything is recorded. **Obligation: budget reservation happens atomically at dispatch rather than as an advisory pre-read, and the mesh's fan-out factor is bounded by config rather than by hope.**

**H4 — Caller identity is hardcoded.** Both `ToolDispatcher::from_registry` call sites in the Assistant pass `caller_pane_id` as literal `0`; the broker request carries `app_id: "assistant"`; and `HostEvent::AgentTurn` records `pane_id: None`. Every head is therefore the same caller in the ledger, in the audit trail, and in tool-dispatch logs. This is the same finding as the per-app budget bucket above, seen from the attribution side: **per-head identity is a precondition for H3's fix, for §3's drain, and for any claim that the audit trail is attributable.**

**H5 — Nothing cancels a turn when its viewer goes away, and today's cleanup is accidental.** A completion send is discarded with `let _`, and the current safe-ish behavior — a dropped receiver causing a permission denial — is a side effect of the pane owning the channel. Move ownership to the head, as §2 requires, and that accident disappears with nothing deliberate in its place. **Obligation: turn cancellation becomes explicit and head-owned, using the `CancelToken` seam that already exists in `src/plexi_ai/mod.rs` for exactly this purpose.**

**H6 — Workspace matching does not canonicalize, and the grant plane does.** `snapshot_for_caller` compares roots by raw path components, while the grant plane canonicalizes (`src/broker/mod.rs` resolves through `canonicalize` before recording a decision). The two planes therefore disagree about what "the same workspace" means the moment a symlink, a `/var` versus `/private/var` prefix, or a git worktree is involved — and the tool list comes back silently empty rather than erroring. Worse for this design: **a nested context root structurally never matches its parent's workspace, so a per-context head can see zero tools from its own workspace.** **Obligation: one canonicalization rule shared by both planes. This is a v1-shaped correctness bug that exists today without any mesh, and §2 makes it unavoidable rather than creating it.**

**H7 — The pane snapshot in the system prompt is machine-global.** Every turn's prompt carries a snapshot of panes unfiltered by context or workspace, so §2's relevance boundary does not exist at the prompt layer at all — the model sees the whole machine regardless of which head it is. The dirty-check is pane-count-only, so a same-frame swap of one pane for another leaves a stale snapshot in place. **Obligation: the snapshot is scoped to the head's context before it reaches the prompt, and its dirty-check keys on content rather than cardinality.** Until then, no statement about a head's relevance boundary is true where it matters most.

**H8 — `build_context_prefix` slices UTF-8 by byte offset and reads the event log off disk.** It truncates with `&line[..200]` and `out.truncate(8000)`, both byte-indexed into a `String`, so a multibyte character on either boundary panics the turn. It also opens `events.jsonl` directly every turn — which means the broker is already violating §3's typed-recall rule today, from inside the layer that rule was written to govern. **Obligation: character-safe truncation, and event-log reads move behind the typed recall surface §3 requires.**

**H9 — Every shared lock is `.unwrap()`ed.** The global registry and pending-call mutexes are unwrapped at every acquisition, so a single panicking worker poisons the mutex and bricks tool dispatch for the entire mesh for the life of the process. One head's bug takes down every head. **Obligation: poisoning is handled rather than propagated; a panicked worker degrades its own head and nothing else.**

**H10 — There is no per-head model plane.** `AiConfig` is cloned once at construction, so configuration changes need a restart, and reasoning effort is silently dropped on `backend = "local"` because the only reasoning path is OpenRouter's nonstandard field. §5's behavioral gate runs against exactly that local backend — **so as written, the gate would assert on a model that never received the head's declared effort, and would pass or fail for reasons unrelated to the behavior under test.** **Obligation: per-head model and effort resolution, honored by the local backend, before the behavioral gate can mean anything.**

Two qualifications on claims made earlier in this document, both from the same audit:

- **Dispatcher snapshots are per-turn**, so mid-turn tool registration is invisible and the real exposure window is a long turn racing pane churn — not the instantaneous race a reader might infer from §2. The snapshot's workspace filter is still the boundary; it is just evaluated once per turn.
- **The conflict-withholding precedent cited in §4 returns `tool_not_found`**, which is indistinguishable from "no such tool ever existed." That is the correct behavior for a model that should not learn a tool exists, and the wrong behavior for §4's routing, where the asker must learn *why* an ambiguous question was routed to a human. Reusing the principle is right; reusing the opaque error is not. **The reason reaches the caller in the ask_question case.**

### Substrate hazards

§3 makes the event log the mesh's memory and §5 puts routines and unprompted messages on top of the notification surface. Both substrates were built for lower stakes than that. Grouped by the section each one lands on.

**The drain (`src/host/event_log.rs`).**

**S1 — `emit` silently no-ops outside the host process.** It writes only when `GLOBAL_EVENT_LOG` is initialized, and the single caller of `init_global` is host startup. A CLI subprocess — which is what a `plexi assistant` invocation is — emits into nothing, and every call *reports success*. **Obligation: a drain write from outside the host either reaches the host or fails loudly. Under §3 a lost write is a pointer to nothing, and a silent one is worse than a crash.**

**S2 — Clean shutdown discards queued records.** Shutdown calls `process::exit(0)` while the detached writer thread still holds up to a full channel of unwritten events, and it never drains. The loss window is widest exactly when activity is highest. **Obligation: shutdown flushes the drain before exit.**

**S3 — Lossiness is unprovable.** `dropped_count` is incremented and never read anywhere, surfacing only in a debug log. **Obligation: dropped records are observable, or the no-loss claim §3 depends on cannot be tested.** This compounds S1 and S2 — three independent silent-loss paths, none of them detectable today.

**S4 — Nothing prunes the log, and every event is written twice** (global and workspace). Drain-as-memory does not solve unbounded growth by itself; it relocates it into two unpruned files per workspace. **Obligation: retention is explicit config, and the double write is either justified or removed.** §3 claims bounded working memory; it must not buy that with an unbounded disk.

**S5 — The only reader is a full-file linear scan** (`read_recent`), already called on every assistant turn, and it silently skips lines it cannot parse — so a torn write becomes a record that never existed. **Obligation: an indexed read addressed by record id, and a parse failure that is reported rather than skipped.**

**S6 — No record ids, no schema version, no unknown-variant tolerance on `HostEvent`.** The first typed reader will fail on every line a newer build wrote. **This confirms and sharpens §3's stated requirement:** ids are necessary but not sufficient — the record format needs a version and forward-compatible deserialization in the same change, or the drain becomes unreadable across an upgrade.

**The event bus.**

**S7 — The bus is volatile and the drain is durable.** `AppTimeline` is an in-memory, uncapped `OnceLock` lost on restart. **Obligation: this document states which data may ride which — transient coordination on the bus, anything a head must remember in the drain.** A head that trusts the bus for memory loses it at the next restart with no signal.

**S8 — Every Assistant shares one subscriber identity.** Subscriptions are registered under a single `ASSISTANT_ACTOR_ID`, so a second Assistant pane wipes the first's subscriptions and whichever pumps first steals deliveries. **This is the same defect as H4 seen from the bus side, and it is fatal to per-head addressing: §2 cannot be built on an identity that is a constant.**

**S9 — Subscriptions match on `app_id` only**, giving cross-context and cross-workspace delivery leakage, and undrained deliveries accumulate without bound. **Obligation: subscription matching carries head identity and context, and undelivered queues have a ceiling.**

**Escalation and notifications.**

**S10 — The escalation path §6 depends on is weak on four independent axes.** Priority is hardcoded below the interrupt threshold at three separate call sites (a recent stint found one of the three, which is itself the finding — the value is duplicated, so fixing one site looks like fixing the bug). `NotifyScope`'s `Context` default means a background head's escalation stays invisible until the user happens to enter that context — the safe default for an app is the wrong default for an escalation. No live socket returns a failure with no queued fallback, so an escalation raised while the host is down is simply lost. And a restart restores pending notifications tombstoned with no response channel, so a caller blocked on a choice can never be answered. **Obligation: escalation is a first-class delivery with one priority source, a scope chosen by urgency rather than inherited from app defaults, durable queueing across host restarts, and a response path that survives one.** §6 rules that an escalation missing a required field is not raiseable; that is unenforceable on a channel that can drop the whole message.

**Routines (§5).**

**S11 — The scheduler's semantics do not yet fit an agent target.** Cron routines have no missed-tick catch-up, so a host closed at 9am silently skips the day, while interval routines all fire at once on startup — the two failure modes point in opposite directions and neither is what a person expects from "check this every morning." `fire_routine` can only spawn terminal panes, and an unscoped routine targets whatever window happens to be active. The overlap guard's liveness fact is "the terminal exited," which has no analogue for a head. **Obligation: §5's head-addressed routine defines catch-up policy, targeting, and its own liveness fact explicitly — none of the four transfers from the pane-spawning path.**

### Assistant-app hazards

The broker and substrate hazards above are about layers the Assistant sits on. These are in the Assistant itself, and several are structural obstacles to sections of this document rather than defects to schedule around.

**A1 — A paneless head cannot answer a host tool call, by construction.** Host-tool dispatch is drained by a viewing pane; when the pane is gone the parked drain handles notifications only, and every host tool call returns the literal `host_tool_failed: assistant pane closed` — forever, not once. §4's `ask_question` is a host tool. **Obligation: host-tool servicing belongs to the head and runs off the frame loop.** With H2, this is the same wall approached from two sides: the seam has no timeout *and* no servicing when unattended.

**A2 — `Turn` has no id and no author, and placement is a positional index.** A turn carries role, text, timestamp, and optional status, thoughts, and detail — no identity, no author — and rows are addressed by mutable position. §8's receipts need to say *who* the counterpart was, and §3's drain-to-transcript projection needs a stable join key. **This is a structural obstacle, not a rendering detail: receipts cannot be built on the current `Turn`.** Obligation: `Turn` gains a stable id and an author before §8 is implementable.

**A3 — `write_turns` rewrites the whole transcript file.** That is correct today, because one pane owns the file and turns finish out of submission order. It becomes destructive the moment §8's canonical conversation has more than one writer. **Obligation: this is the mechanism behind §8's one-writer rule — the rule is not stylistic, it is the only safe use of the current persistence model, and any move to concurrent writes requires replacing whole-file rewrite first.**

**A4 — `state.toml` is an unlocked read-modify-write.** A save reads, mutates, and rewrites the whole file, so a write from one context reverts another context's pointer. With per-context heads writing independently this stops being a rare interleaving and becomes routine. **Obligation: per-head state is separately addressable, or writes are serialized through one owner.**

**A5 — At most one headless Assistant can exist machine-wide.** `background_apps` is keyed by app *type* id, so the headless slot holds one entry for the Assistant and no more. §2 wants a head per context root plus a global head. **Obligation: headless instances are keyed by head identity, not app type.**

**A6 — Conversation claiming fires on every activation, and the global head has no home.** `set_active_conversation` stamps the context claim on every call rather than once, and a global head — belonging to no context — collapses onto the `$HOME` store, which is also where every rootless context already lands. §8's one-canonical-thread ruling removes the claiming mechanism entirely; the global head's storage location is a separate question that ruling does not answer. **Obligation: the global head's store is addressed explicitly, never by falling back to a home directory that other things also fall back to.**

**A7 — Model and view are already split, and split wrong.** State is divided between `AssistantModel` and a per-view `AssistantApp` such that the permission reply channel exists in only one view while the composer echoes across views. §2's head/view split is therefore not a clean new seam over an undivided object — it is a *re-cut* of an existing bad seam, and the existing division has to be understood before it is redrawn. This is why H5's cancellation work and the split cannot be sequenced independently.

**A8 — The audit log's actor is hardcoded.** `audit.jsonl` records `agent:assistant` regardless of which head acted — the same defect as H4 and S8, now on the third plane. Three independent identity surfaces all collapse every head into one string. **Obligation: one head identity, threaded through broker attribution, bus subscription, and audit actor in the same change.** Fixing any one of them alone leaves the audit trail unattributable.

### Other systems this reaches

**Event log schema.** `HostEvent::AgentTurn` keys on `pane_id: Option<u64>` — the Assistant's existing event is pane-addressed, and a headless head has no pane. Supersession: assistant records carry head identity and `pane_id` becomes what it always should have been, optional provenance. The rest of the drain's obligations are S1–S6 above.

**Notify.** `notify_cli` takes `source_context_id` and `source_pane_id` and falls back to `unwrap_or(0)` for the pane. The documented rule in `src/cli/AGENTS.md` is that an unresolvable sender identity *widens* the effect — notify escalates to global scope rather than fabricating a home. A pane-less head raising an unprompted message (§5) lands on exactly that path, and the widening that is safe for an unknown sender is wrong here: a context head's message belongs to its context. Supersession: a head is a first-class notification source with its own identity; the widening rule stays for senders that genuinely cannot be resolved. Delivery reliability is S10.

**Command palette.** Covered in §8. `palette_agent_rows` builds `AgentRow` from `window_id` and `pane_id`; heads have neither.

**The agent-definition registry.** `AgentRegistry` and the `AGENT.md` format are the natural carrier for the declared half of §4's capability card. But today they describe personas the Assistant can switch between within one pane (`active_agent_id`, defaulting to `"default"`), which is a different axis from a head. Whether a head's persona set and the head's own identity share this registry is an open decision, not something to settle by implementation.

## Non-goals

- **No isolation claim.** Heads are a relevance and addressing structure. Sandboxing arrives with the WASM platform phase; nothing here narrows the authority plane.
- **No memory search infrastructure.** Recall resolves pointers and filters records. Embeddings, ranking, and a vector store are not in this design and would violate the portable-format commandment if added carelessly.
- **No new inter-agent transport, and no foreign one on the inside.** `ask_question` is a host tool; structured cross-agent data rides the event bus; heads never talk over PTY injection. The distinction that governs §7 is *where the boundary is*: a standard wire protocol is the right thing at the edge where an outside party is on the other side — MCP Apps' postMessage bridge is adopted exactly there and nowhere else — and the wrong thing between two of the host's own heads. A2A and ACP compatibility, if ever wanted, is likewise an edge bridge and never the internal wire.
- **No multi-user and no cloud dependency.** A head is local. The portable-instance phase may carry it later; nothing here may require it.
- **No autonomous authority escalation.** A head cannot grant itself, or another head, anything. The user remains the only source of new authority.

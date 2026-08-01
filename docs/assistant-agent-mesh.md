# Assistant Agent Mesh

Status: active.
Stint: none yet — filed after the stable v1 cut lands.
Parent: [`assistant-host-app.md`](assistant-host-app.md).
Authority plane: [`assistant-authority-model.md`](assistant-authority-model.md).
Last updated: 2026-08-01.

This document defines the destination shape of the Plexi Assistant as a **mesh of head agents** rather than a single chat surface: one head per context root plus one global head, a shared central record they all write to and hold pointers into, a typed question channel between them, and an addressing scheme that makes every head individually testable.

It owns exactly one plane: **how multiple Assistant instances coexist, remember, and talk** — and, in §7, the one third-party surface that plane must terminate cleanly: MCP Apps, where an outside server hands a head a UI. It does not restate the Assistant's product surface (agents, skills, slash commands, settings, UI — [`assistant-host-app.md`](assistant-host-app.md)), the authority plane (threat model, reference monitor, grant binding — [`assistant-authority-model.md`](assistant-authority-model.md)), or host-owned workflow execution ([`agent-run-orchestration.md`](agent-run-orchestration.md)). Where this design needs a fact from those documents it cites them.

## Sequencing

This is post-v1 work. [`2026-08-01-v1-cut.md`](2026-08-01-v1-cut.md) rules the Assistant, orchestration, browser, marketplace, native WASM distribution, and media editing out of the stable v1 sprint, and lists `agent-run-orchestration` under Post-v1. `NORTH_STAR.md` places the Assistant-as-workspace-operator in Phase 3 — Intelligence. Nothing in this document may be pulled forward into the v1 acceptance set.

Two hard prerequisites are not matters of scheduling, and both block work rather than merely preceding it. One is a sandboxed webview pane, without which §7 cannot begin at all; it is stated in full there. The other is root uniqueness.

A head agent addressed by its context root only exists if roots are unique. [`context-root-uniqueness-and-rollup.md`](context-root-uniqueness-and-rollup.md) records that duplicate roots are not merely possible today but guaranteed by two creation paths, and that two contexts sharing a root already collide on context-scoped app state. Per-context heads inherit that collision exactly — two heads at one root are two agents that believe they own the same working memory. The uniqueness ruling in that brief is a blocking dependency for this one.

## 1. What exists today

The Assistant is a builtin host app instantiated per pane. `AssistantApp::new` takes a `workspace_root`, a broker, a config dir, and a `context_id`, and `src/assistant/store.rs` already keys conversation state per host context: `state.toml` carries `[contexts.<context_id>]` tables holding each context's active conversation, session name, agent, and effort, while transcripts under `conversations/<id>.jsonl` are shared workspace-wide and claimed by the first context to activate them.

So per-context *conversation state* exists. Per-context *agents* do not. The agent is a property of the pane: two Assistant panes in one context are two independent agents with no relationship, and a context with no Assistant pane has no agent at all. There is no host-level Assistant that outlives its pane, and no `plexi assistant` CLI namespace — the CLI exposes `plexi ai`, `plexi agent`, `plexi routine`, and `plexi notify`, but nothing addresses an Assistant instance directly.

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
- More than one head claims it → route to the human, naming the claimants. This is the `snapshot_for_caller` conflict rule applied to a second surface: when ownership is ambiguous the correct behavior is to fail visibly to a person, never to pick a winner silently.

**Human routing uses the notification surface** with its existing scope semantics, not a new channel.

**An answer is information, never authority.** `ask_question` returns data. It cannot make another head act, and a head cannot use it to reach a tool its own reference-monitor check would deny. Asking a head that can do something is not a way to do that thing — that is the confused-deputy path the authority model exists to close, and the mesh must not reopen it.

**Bounds.** Every ask carries a deadline. A question that would close a cycle — A asking B while B is blocked on A — is refused at the handler with the cycle named. Ask, answer, refusal, and timeout are each drain records.

## 5. Individually testable, and what ships because of it

**Every head is addressable by direct message.** A `plexi assistant` namespace addresses a head by root or `--global` and delivers a message to it, returning its reply. This is the fourth commandment applied to agents — if a head is not reachable from the CLI it does not exist for an agent — and it is simultaneously the test harness: a head that can be messaged in isolation can be tested in isolation, with no pane, no window, and no sibling heads.

**Two test tiers, both required.**

*Plumbing, no model.* The inert-broker harness (`HostHarness::add_assistant_pane`) proves head creation and lifetime, addressing, drain writes and pointer resolution, question routing including the ambiguous and cyclic cases, and scope enforcement — all without a model, all deterministic, all in the default suite.

*Behavior, live model.* A head's actual conduct — that it asks rather than guesses, escalates in the required shape, speaks unprompted only when the filter says so — cannot be proven by an inert broker. Those tests run against a live model through `LocalOpenAiBackend` with `backend = "local"` pointed at a local Meridian proxy (stint 0579). They assert on **observable structure**: which record kinds appear in the drain, which target an ask resolved to, whether an escalation carries every required field. They never assert on model prose, because prose is not a contract.

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

## Non-goals

- **No isolation claim.** Heads are a relevance and addressing structure. Sandboxing arrives with the WASM platform phase; nothing here narrows the authority plane.
- **No memory search infrastructure.** Recall resolves pointers and filters records. Embeddings, ranking, and a vector store are not in this design and would violate the portable-format commandment if added carelessly.
- **No new inter-agent transport, and no foreign one on the inside.** `ask_question` is a host tool; structured cross-agent data rides the event bus; heads never talk over PTY injection. The distinction that governs §7 is *where the boundary is*: a standard wire protocol is the right thing at the edge where an outside party is on the other side — MCP Apps' postMessage bridge is adopted exactly there and nowhere else — and the wrong thing between two of the host's own heads. A2A and ACP compatibility, if ever wanted, is likewise an edge bridge and never the internal wire.
- **No multi-user and no cloud dependency.** A head is local. The portable-instance phase may carry it later; nothing here may require it.
- **No autonomous authority escalation.** A head cannot grant itself, or another head, anything. The user remains the only source of new authority.

Status: active
Stint: none yet

# One permission gate (P1)

Phone and relay passages describe deferred design ideas. The former phone client and relay were removed from V1; a future plugin requires a separate design and qualification.

## Purpose and product story

Plexi lets Ian talk with several team leads at once, queue work that runs without
an open conversation pane, and return to decisions and attributable results.
YouTube, Narrative, DU, and Plexi each have a lead lane and a canonical thread.
A shared queue shows what the host actually started; a small “Needs you” inbox
shows decisions that unblock it. Assignments distinguish **system** work that
improves the environment from **output** work that ships a project result.
Chess proves the permission boundary; text and video editing prove that a person
can inspect, correct, accept, and undo an agent's contribution in the same app.

This document owns the cross-surface Agents API and its proof sequence. It is a
destination contract, not an implementation tracker. All types, new verbs, test
names, and scene names below are **proposed**, unless explicitly marked existing.
The phases here are local product phases; they do not renumber the cloud plan.

### Inputs and ownership

The approved architecture package was read as decisions, with source claims
rechecked against this worktree at `21b6f4aa`. The requested `file:line` evidence
below is a review snapshot of that revision, not an instruction to locate future
implementation by line number. Symbols are included for subsequent navigation.

| Input | Decision carried here |
|---|---|
| Architecture package: existing inventory (Grok Bot, 2026-10-05; not in repo) | Reuse shipped seams; distinguish declarations, enforcement, and runtime evidence. |
| Architecture package: alignment (Grok Bot, 2026-10-05; not in repo) | Every numbered drift has a disposition in the drift table. |
| Architecture package: Munder lessons (Grok Bot, 2026-10-05; not in repo) | Typed identities, receipts, assignments, and one host-owned control plane; MCP is an adapter. |
| Architecture package: agent-team model (Grok Bot, 2026-10-05; not in repo) | Durable heads, narrowing subagent runs, broad read/narrow write, explicit full trust. |
| Architecture package: journeys (Grok Bot, 2026-10-05; not in repo) | Directory lead, unconfigured folder, plugin launch, phone approval, creative editing, and multi-lead shell. |
| Architecture package: proof plugins (Grok Bot, 2026-10-05; not in repo) | Chess before creative editing; this document's P1–P5 order governs these proofs. |
| Architecture package: Astra recommendation (Grok Bot, 2026-10-05; not in repo) | Mandatory dispatch gate, exact-call binding, host-approved Chess, parameterized harness proof. |
| Architecture package: cloud specification (Grok Bot, 2026-10-05; not in repo) | §4.2 intake, §4.4 allocation tree, §4.5 events; E2E-B delegation shape. |

The architecture package lives outside the repo, and its decisions are restated below.

The external package is design provenance. Implementers should use the contracts
below and the owning repository documents, rather than require those external
absolute paths at runtime or reproduce their claims as product documentation.

| Repository owner | Integration / explicit supersession |
|---|---|
| [NORTH_STAR.md](../../NORTH_STAR.md) | Portable records, CLI completeness, host enforcement, local first, and correction/undo remain constraints. |
| [Root AGENTS.md](../../AGENTS.md) | HostModel owns logic; structured coordination uses the event bus; binary media uses typed pipes; PTY injection is human control. |
| [Authority model](../assistant-authority-model.md) | Retains threat model and reference monitor. Proposed default-profile amendments are isolated below and require Ian's sign-off. |
| [Testing](../../src/testing/TESTING.md) | Owns implementation build gates, Rust/scene split, isolation, editor gate, and mandatory visual review. |

Cloud §4.2/§4.4/§4.5 contracts are incorporated here for local execution; its
HTTPS relay, daemon, hosted runtime, tenancy and multimedia rollout remain with
that plan. Its older uncertainty about locating Chess is resolved by evidence E8.

## Glossary

| Noun | Meaning and boundary |
|---|---|
| Principal | Authenticated human, device, agent, or application identity. Origin is stamped by the host, never asserted in message text. |
| Agent / head | Durable opaque agent ID plus definition. A head owns a team-facing canonical conversation; an Assistant pane is a view. A selected prompt/persona is not itself a new head. |
| Team | Context-root binding, head and member references, declared budgets and policies. Membership conveys relevance, not authority. |
| Assignment | Goal, owner, resource/action scope, deadline and budget; required `kind: system\|output`, optional `pays_back`. |
| Delegation | Host record allocating a subset of an assignment to a child; intersection can preserve or reduce rights, never add them. |
| Run | One supervised execution attempt of an assignment, with observed lifecycle, pinned configuration, reservation and typed result. |
| run token | Revocable host-issued credential bound to principal, agent, run, assignment, delegation and host generation. Possession does not bypass live policy checks. |
| Message + receipt | Attributed speech act and durable delivery/execution state. Information is not delegated authority. |
| Request | Typed unresolved human decision: `needs_input` or `permission`. Discussion, snooze, reading, and approval are different operations. |
| ChangeSet | Prepared delta against a plugin-owned resource, with revisions, footprint, preview, selected hunks and attributed commit/revert receipts. |
| Event / drain | Host-stamped, versioned, addressable record and the lossless append/read service for agent records. Bus delivery and UI projections derive from it. |

A head may own many successive runs. A child run has its own transcript and can
have a view without becoming a standing roster member. A skill supplies
instructions and tools; it is neither a principal nor a grant.

## Verified implementation seams

These are source observations, not evidence that this design has passed a build
or live test. Negative findings are bounded to the inspected implementations.

| Ref | Source in this worktree | Observation |
|---|---|---|
| E1 | `src/plexi_ai/tool_dispatch.rs:187` `snapshot_for_caller`; `:477` `from_registry`; `:513` `namespaced_for` | Discovery uses `evaluate_reach` and host context ownership. Constructors initialize `hooks: None`; raw root equality is not today's boundary. |
| E2 | `src/plexi_ai/tool_dispatch.rs:551` `set_hooks`; `:594` `dispatch_call`; `:644` `dispatch_inner` | Hooks are optional on host and plugin tool calls. Inner dispatch sends the caller string; a snapshot is not a live grant check. |
| E3 | `src/broker/mod.rs:133` `GrantRecord`; `:183` `matches`; `:208` `PermissionRequest`; `:435` `record` | Resource fields exist on grants but are absent from requests/matching/replacement identity. Expiry is checked; duration itself is not enforced by matching. |
| E4 | `src/assistant/mod.rs:149` `AssistantToolHooks`; `:963` `gated_dispatcher`; `:1039` `set_hooks` | Assistant installs the ask hook, keeps session approval by tool name, and auto-allows plugin-asserted read-only tools under Ask. |
| E5 | `src/agent/mod.rs:740` `gated_dispatcher` | AgentHost filters a turn snapshot from broker decisions and does not install the Assistant hook. |
| E6 | `src/app/host_mcp.rs:73` `register_pane_credential`; `:349` dispatcher construction; `:385` tool routing; `:393` `dispatch_call` | MCP bearer binds a pane/context/root. App tools dispatch without a grant hook; event tools have separate service routing. |
| E7 | `src/app/plugin_call.rs:19` `caller_identity`; `:40` context lookup; `:56` dispatcher; `src/app/lifecycle.rs:2305` `CallAppTool`; `src/app/mod.rs:583` `handle_socket_line` | App-call accepts a forwarded pane ID; existence is checked, peer ownership is not established on this path. No pane maps to `user`. Notification peer stamping does not fix plugin-call. |
| E8 | `plugins/chess/chess.py:170` `_tool_play`; `plugins/chess/chess_domain.py:137` `play`; `:158` receipt lookup; `:283` `_authorize` | Host caller string reaches Chess. Seat checks are separate; revision, operation ID, immutable next-game value and receipt/outbox already exist. |
| E9 | `src/assistant/audit.rs:11` `AuditEvent`; `:27` `now` | Audit schema lacks call/grant IDs and stamps the literal `agent:assistant`. |
| E10 | `src/agent/mod.rs:281` `AgentRegistry::load`; `:346` `workspace_agents_dir`; `src/assistant/skills.rs:65` `SkillRegistry::load` | Agent tiers are builtin/user/workspace, with later IDs shadowing earlier ones. Workspace agents use the channel dir; workspace skills use literal `.plexi/skills`. |
| E11 | `src/assistant/mod.rs:2414` agent creation; `src/app/registry.rs:949` `resolve_workspace_root_with_channel` | Agent creation writes the profile tier. Workspace resolution seeks the channel marker and excludes home before checking it. |
| E12 | `src/app/lifecycle.rs:219` `SubmitAssistantTurn`; `src/cli/plugin.rs:1932` `assistant_send_cli` | Text and request ID go to the first matching Assistant pane, optionally filtered by pane/context. No durable agent/conversation addressing here. |
| E13 | `src/host/event_log.rs:278` `emit_scoped`; `src/host/plugin_timeline.rs:285` `AppTimeline`; `:638` `begin_rollback`; `:678` `resolve_rollback_verify` | Event writer drops on full. Timeline is in memory; rollback entrypoints are compiled only for tests. |
| E14 | `src/editor/transaction.rs:72` `Transaction`; `src/editor/history.rs:24` `EditHistory`; `src/app/text_editor_view.rs:1000` `impl App` | Grouped editor transactions exist without actor/run/op tags. The inspected builtin App implementation has no `ExposeTools` or `ToolCall` handling. |
| E15 | `src/app/assistant_host_tools.rs:465` file edit routing; `:1229` `edit_scoped_file` | Assistant editing reaches a direct filesystem string replacement/write, outside editor history and without expected revision. |
| E16 | `src/media/video.rs:119` `VideoDecoder`; `:212` non-macOS implementation; `:694` `MockVideoDecoder`; `src/app/video_player_view.rs:27` `new` | Existing player uses the decoder service. Production non-macOS open returns `NotImplemented`; a procedural mock exists for tests. |
| E18 | `src/testing/mod.rs:160` `HostHarness`; `:194` `new`; `:411` `hidden_frame`; `:544` `inject_ipc`; `:639` `wait_for_app_event` | Harness provides the production host seam, isolated profile and hidden-frame/event observation helpers. |
| E19 | `src/cli/agent.rs:793` `registerPlexiHostMcp`; `website/railway.json:1` | Pi registers the host MCP endpoint from its pane environment. Railway configuration exists for the website; this is not a phone relay. |
| E20 | `src/cli/args.rs:273` Assistant `Send`; `:570` App `Open`; `:631` App `Install`; `:883` App `Call`; `:897` Host `Start`; `:956` Host `Screenshot`; `:1087` Pane `List`; `src/mcp_http.rs:38` route | Existing command spellings and `/mcp` endpoint used in the P1 demonstration below. |

## Agents API vs host MCP

The Agents API is a host-owned service layer above individual tool operations.
MCP, CLI/socket, the in-process Assistant, phone transport, and later A2A adapt
onto it. Every operation requiring authority resolves through the same monitor;
all tool execution crosses the mandatory gate in `ToolDispatcher::dispatch_call`.
The API owns records and supervision; transports own parsing and authentication.

| Reason | Consequence |
|---|---|
| A pane bearer is an ephemeral identity | Durable agents, assignments and runs are independent of pane creation and destruction. |
| Tool RPC is not the entire work model | Receipts, conversations, pending requests, projections and resumable drains are first-class host operations. |
| Several ingress paths already exist | Putting policy only in MCP would leave in-process and socket calls with separate enforcement. |
| Transport versions evolve | Versioned host schemas remain stable behind explicit adapter migrations. No external protocol claim is required for core correctness. |
| Remote peers differ from local members | A2A is a future authenticated edge bridge, never the local scheduler, permission store or internal bus. |

MCP tools such as `plexi__assign` and `plexi__delegate` invoke the host service.
The in-process Assistant calls it directly; it does not loop back over HTTP.
Internal structured coordination stays on the event bus; the drain provides
persistence/replay underneath delivery. Neither mailbox files nor PTY injection
become an agent messaging transport.

This supersedes the Host Assistant backend section's wording that grants are
evaluated “at the MCP edge”: authentication happens there; authorization happens
below it. MCP remains the external-agent tool adapter. CLI commands are public
human/automation views of the same operations, not a new agent RPC protocol.

## Authority defaults and amendments

| Head/profile | Proposed default policy | Never implied |
|---|---|---|
| Global head | Audited broad reads of ordinary home files and permitted cross-context summaries, represented by a visible baseline read grant. | Secret access, terminal scrollback, writes, sends, process execution, plugin control, or blanket export of home content to a model. |
| Directory head | Reads default to its selected root; write scope offered for an assignment defaults to that root or a narrower selection. | Root discovery or `team.toml` alone granting writes. |
| Member / child | Only the parent-authorized intersection, including read scope and budget. | Inheritance of the global head's full visibility simply because it is the parent. |
| Full trust | Explicit opt-in per head; a named, visible grant profile for broad reversible work, with persistent desktop/phone indicator and revoke action. | Impersonating the human, sharing tokens, bypassing deny rules, or suppressing irreversible-action approval. |

Broad read is an authored product default requiring Ian's policy sign-off, then
visible user consent when enabling that head profile. Consent creates an
inspectable baseline grant; it is not a hardcoded bypass. Existing heads do not
silently gain it. Directory-head onboarding likewise proposes a read root;
write authority starts only with an approved assignment or explicit grant.
An unconfigured folder asks what it is for and offers a scoped team definition.
It never falls back to treating home as a project.

**Amendments requiring Ian's sign-off:** the authority model's *Filesystem
authority* “context root is proposed, not ambient” rule remains true for writes
and root discovery. Its default read behavior is amended to allow the named
baseline read profile. *Context isolation* and *Model context* are amended only
for intentionally granted global-head reads across contexts; directory heads
retain context-local discovery. The prohibition on unbound workspace-global
Assistant grants is amended for this explicit read profile and opt-in full trust,
not for `workspace_root = None` wildcard matching. Bind a global policy to an
explicit host/user domain and concrete resolved resources on every call.

The authority model's *Process execution* exact-command rule is extended only
for an approved assignment/profile that enumerates bounded command families.
Each invocation still resolves exact argv, cwd, environment names and footprint
before gating. No `*` shell grant is inferred from “full trust.” Unbounded native
shell authority is not qualified by P1–P5. Its destructive-command rule survives.

A full-trust profile lists its included reversible capabilities; secrets and
external sends remain separately granted. Publication, sending, deletion and
other irreversible effects always require a fresh exact-action permission
request, including exact destination/content where relevant. Approval cannot be
relayed by another agent. Denies take priority over any profile.

Host classification, not a plugin's `read_only` claim, decides risk. Unknown tools
ask. Broad read excludes credential stores, known secret files and terminal
output by default; a redactor is a backstop, not proof arbitrary files contain
no secrets. Read authorization and remote model transmission are distinct:
provider-bound content is minimized and subject to the configured egress grant.
The profile UI makes this distinction visible.

**Runtime limit:** host checks constrain host-mediated calls. They cannot
constrain a CLI agent that disables/bypasses its own sandbox and reads or writes
through native OS syscalls, nor prove that a malicious plugin's claimed footprint
is honest. A cwd or git worktree is not OS isolation. Untrusted code/workflows
run in a qualified VM/sandbox with explicit mounts, network and secrets policy.
P2 may supervise trusted CLI work but must not advertise a native isolation
boundary it has not qualified. The authority model retains ownership of the
command-worker sandbox and secret-injection implementation.

### Human-owned pane authority and context scoping

1. **Human-owned pane definition:** a human-owned pane (`TerminalPane::is_human_owned` / `PlexiApp::is_human_owned_pane`) is a terminal pane whose session the host spawned directly for the user, with no agent harness/role, no parent pane (`parent_pane_id`), and not spawned by an agent (`agent_spawned == false`). Any pane launched with `--agent` or spawned from an agent pane (including a plain shell created by an agent via `plexi pane new`) is marked `agent_spawned = true` (`PLEXI_AGENT_SPAWNED=1`) and never qualifies as human-owned.
2. **User authority scoped to the pane's context root:** a human-owned pane carries the user's authority by default within that pane's context root (`PlexiApp::pane_context_authority_root` / `user_context_authority_grants`). Creating an agent from a human-owned pane in its own context (`create_agent`) succeeds immediately without filing a `needs-you` permission request.
3. **Spawned agents are still narrowed by grants:** the parent-subset check in `create_agent` runs against the human pane's context authority, so the created agent receives **only** the grants explicitly declared on `create_agent`. An agent cannot exceed its declared grants at tool-call time or when delegating/creating a sub-agent. An agent-spawned pane has no ambient authority and still requires pane-session grants (`pane_session_grants`) via desktop approval.
4. **Subdirectory contexts are isolated units:** a pane in context A has no authority over context B, including when B is a nested context or nested workspace inside A's directory tree. `agents_api` operations (`create_agent`, `list_agents`, `list_grants`, `spawn_run`, `delegate`, `call`) and secret operations (`secret get`, `secret list`, `secret set`, `secret rm`, `secret read`) targeting another context root are refused with `permission_denied`.
5. **Not an OS file-access boundary:** this authority model governs the Plexi API (`agents_api` / host tools), secrets, and grant scoping. It is **not** an OS file-access boundary: a process running in the pane still executes as the OS user and can read or write any file on disk that the OS user can access.

## P1 — One permission gate, resource-scoped grants, Chess

### Goal, scope, non-goals

Prove one human approval authorizes one exact attributed move, independent of
which ingress proposes it. Close optional-hook and socket identity bypasses.

Scope includes plugin and host tools, Assistant and AgentHost, host MCP, socket
plugin-call and CLI; resource/argument-bound grants; one pending permission service;
structured errors; correlated audit; host-approved Chess mutations.
Discovery remains a filter, never authorization to invoke cached tools.

Non-goals: durable teams/delegation, creative undo, generic role manifests,
remote approval, relay deployment, Pi loop-driver implementation, universal
native-process confinement, or a claim of lossless general event history.

### Gate and proposed data shape

```rust
struct PermissionRequest {
    call_id: CallId,                  // host UUID, not provider tool-call ID
    principal: PrincipalId,
    actor: ActorBinding,              // type, opaque id, trust origin
    origin: HostOrigin,               // context + optional pane; host supplied
    workspace: WorkspaceBinding,     // explicit root/domain, never wildcard None
    target: TargetBinding,           // action + package + instance + generation
    resource_scope: ResourceScope,
    resource_id: ResourceId,          // game, doc, path handle, account, etc.
    args_fingerprint: Digest,         // normalized resolved operation
    session_id: SessionId,
    run_id: Option<RunId>,            // required for agent work in P2
}
struct GrantRecord {
    grant_id: GrantId,
    binding: GrantBinding,           // same identity fields as request
    decision: Decision,
    lifetime: GrantLifetime,         // Once(call), Session(id), Until(time)
    expires_at: Timestamp,
    revocation_epoch: u64,
    source: GrantSource,
}
struct AuthorizedCall {              // opaque, constructible only by monitor
    request: PermissionRequest,
    resolved_args: CanonicalValue,
    grant_id: GrantId,
    grant_epoch: u64,
    resource_handle: ResolvedResource,
}
```

`GrantRecord::matches` compares resource scope and ID, actor (including trust
origin), target/provider identity, context/workspace binding, normalized arguments,
session/run restrictions and expiry. Duration is executable policy: once is
consumed atomically, session matches one live session, expired/revoked never
matches. Grant replacement uses this same identity; approving another resource
must not erase or widen an existing grant. Persisted and session approvals have
the same binding, differing only in lifetime.

Normalize after schema validation and resource resolution: sort object keys,
reject duplicate keys/unknown authority fields, preserve typed values, and
normalize only schema-defined equivalents. Include operation ID, revision,
resource generation, destination, argv/cwd and environment **names** where
applicable. Hash a versioned canonical representation. Never hash secret values
into model-visible records; carry a host secret-binding reference instead.
Whitespace in prose or command arguments must not be silently normalized away.

Proposed `dispatch_call` execution sequence:

1. Authenticate adapter context; reject missing/stale credentials and forged
   origin claims before resource disclosure. Allocate a globally unique call ID.
2. Resolve the registered target, live package/instance generation, host risk
   class, concrete resource and validated arguments. Reject invisible targets.
3. Ask the mandatory monitor for a decision against live policy. No constructor
   can produce an executable dispatcher without its monitor handle.
4. On Ask, persist an exact pending request and return `permission_required`.
   A host decision resolves that request; clients cannot substitute arguments.
   Click approvals, agent questions, and blocked runs are one host record.
   `plexi needs-you` lists and resolves it; the desktop badge and the phone
   `/api/needs-you` route read those same rows.
5. On approval/resume, recheck identity, expiry, revocation and resource binding;
   then send an opaque `AuthorizedCall` to the selected adapter. No fresh ambient
   lookup may redirect the operation after authorization.
6. At mutation, verify resource preconditions and the live authorization epoch.
   Serialize commit admission against revoke: whichever wins defines the boundary.
7. Record use and outcome with call/grant IDs, actor, operation and revisions.
   Fail before mutation if required intent/audit persistence fails.

Unknown tools can return `tool_not_found` without leaking hidden resource IDs;
that is a refused dispatch, never an ungated execution. Authorization and
resource-precondition failure do not consume an unrelated one-shot grant.
A used one-shot approval can only recover the receipt for its original operation.
It cannot authorize a second mutation.

`set_hooks` becomes observation/presentation only. Remove its permission semantics,
Assistant tool-name session sets, and AgentHost's snapshot-only authorization.
Host tools and Agents API control operations have registered handlers under the
same dispatch gate. App launch and subscription handlers consume the monitor's
authorization, rather than independently consult `permissions.toml` or consent
state. Direct CLI/socket variants route to these same registered operations.
Human UI-originated operations have a host-stamped human principal and explicit
interaction intent; a socket client cannot manufacture that intent.

Lower WASM import/capability checks remain defense in depth and enforce the
same granted scope. “One gate” removes competing policy decisions; it does not
remove domain validation, runtime containment or conditional commit checks.
A launch declaration, role preset or purchased workflow requests capabilities;
it cannot grant them. Existing launch permissions must migrate to the same store.

### Socket identity and result protocol

Resolve `CallAppTool` origin using host-captured peer credentials/ancestry tied
to a live pane, or a host-issued scoped session credential. A supplied
`caller_pane_id` is a claim to validate; existence is insufficient. If a verified
peer disagrees, reject rather than silently use either claimed identity.
No-pane callers require explicit authenticated sessions and context selection;
absence of a pane is never `user`. Unsupported peer verification fails closed
or uses a credential, never an environment-variable fallback.

```json
{
  "schema_version": 1,
  "call_id": "call_opaque",
  "error": {
    "code": "permission_required",
    "state": "needs-grant",
    "pending_request_id": "req_opaque",
    "retry": "resume_exact_request"
  }
}
```

| Result | Meaning / client behavior |
|---|---|
| `permission_required` | One persisted pending ID, no mutation; display/subscribe to its host decision. Repeated identical pending call does not create another prompt. |
| `permission_denied` | Terminal refusal, including forged identity/revocation; no fallback to another actor. |
| `stale_revision` | Authorized resource changed; Chess requires reread/replan, not another permission click on the stale payload. |
| `edit_conflict` | Prepared creative edit cannot apply to current state; P3 adds compensating preview. |
| `operation_conflict` | An existing operation ID was reused with different payload/actor. |
| `outcome_unknown` | Execution may have happened; reconcile receipt before retry, never claim success from a timeout. |

Errors remain typed through Rust result, protocol/WIT, SDK `ToolCall`/`ToolResult`,
MCP tool result and CLI JSON output. Human-readable detail is supplementary.
Permission-required is not a model exception to parse for text instructions.
Authentication failures can use transport errors but map to a named denial in
public CLI/API results without revealing foreign resources.

### Chess adaptation and audit

`_tool_play` receives a host-stamped actor and immutable authorization envelope
beside its arguments; `_authorize` validates that host authorization for this
instance/game/side. Remove private agent-seat enrollment as a prerequisite and
remove `--white`/`--black` authorization setup. White/Black remain chess domain
roles and optional display labels; host resource scopes can limit a grant to a
side. The authenticated local board interaction remains a human move.

Keep `expected_revision` and `operation_id`. Check stale revision before deriving
a new side requirement for a pending move: a human correction must report stale,
not misleadingly demand a Black seat after it became White's turn. Keep legality,
game generation, promotion and terminal-game validation in the app.
An approved call is not permission to falsify a legal move or revision.

Retain atomic board/receipt/outbox persistence; qualify crash behavior rather
than infer it from an in-memory `PlayOutcome`. Duplicate operation with the same
actor and payload returns its original receipt without a second move/event.
Changed payload is `operation_conflict`. Revoked/foreign callers cannot obtain
another actor's receipt through the mutation path; authorized receipt lookup is
separate. A lost transport response does not justify minting a new operation ID.

Persist `grant`, `use`, `outcome`, `deny`, `ask` and `revoke` records with actual
principal/actor, call ID, grant ID, resource identity, argument fingerprint,
operation ID and before/after revisions. P1 supplies durable correlated audit
writes; P2 folds them into the general lossless drain without a second authority
log. Outcomes after a crash can be reconciled from plugin receipts, or explicitly
unknown. Log info-level boundaries without recording tokens or secret payloads.

### Migration

Supersede optional hooks and legacy launch/subscription decision stores outright.
There is no “legacy caller” bypass, compatibility dispatcher, or seat setup shim.
Update SDK/protocol consumers together; mismatched versions reject explicitly.
Default proposal: invalidate old broad grants, discard session approval caches,
then ask in the exact-call form. A migration prompt may issue new narrow grants
only after Ian signs off on that alternative; it never silently translates them.
P1 uses authenticated session/agent bindings; P2 replaces their temporary agent
addressing with durable IDs and revokes superseded pane-only credentials.

### Acceptance criteria

1. **AT-P1-01 — One parameterized HostHarness test:** through Assistant, MCP and
   socket ingress to real Chess, no grant prevents mutation; an exact approved
   move commits once with attribution; a concurrent human move makes the pending
   move stale; wrong actor/resource, changed arguments and revoked grants fail;
   duplicate operations cannot move twice. Use scripted Assistant output.
   Verification: proposed Rust test `permission_gate_real_chess_all_ingresses`,
   parameterized by ingress and adversarial case, loads `plugins/chess` through its
   real runtime. Exercise the production Assistant turn loop, MCP parser/auth
   binding and socket identity handler, not a direct call to `chess_domain.play`.
   Assert the actual board, receipt, audit linkage and unchanged revision on
   denial; mutate the board through the real human input path while approval waits.
2. **AT-P1-02 — Exact grants and duration:** Rust table test
   `grant_binding_and_lifetime_matrix` varies every binding field independently,
   JSON key order, semantically changed values, expiry, session and once reuse.
   A revocation/commit race test proves no new commit starts after revoke wins.
3. **AT-P1-03 — No constructor or host-tool bypass:** HostHarness test
   `all_dispatchers_require_monitor` covers Assistant, AgentHost, host MCP,
   plugin-call, native host tools, plugin launch and event subscription. Remove a grant
   after discovery; invocation still refuses. An plugin claiming `read_only=true`
   cannot suppress a host-classified mutation prompt.
4. **AT-P1-04 — Socket provenance:** Rust socket-ingress fixture
   `app_call_rejects_forged_or_missing_identity` submits another live pane ID,
   nonexistent pane ID, absent pane, stale credential and mismatched peer/session.
   Assert typed denial and no Chess mutation; an authenticated no-pane session
   can request permission for its explicit context without becoming the human.
5. **AT-P1-05 — Recovery and failure shape:** HostHarness test
   `chess_receipt_survives_lost_reply` injects failure around persistence and
   publication, retries the same operation, and observes one mutation. Rust
   adapter round-trip tests preserve structured errors through SDK/WIT, MCP and
   CLI serialization; audit I/O failure before execution prevents the move.
6. **AT-P1-06 — Observable approval:** TOML scene
   `agents-chess-approval.toml` shows one pending move, exact actor/resource/move,
   approval, attributed result and stale-state message with realistic board data.
   Use semantic assertions and screenshot review. Add a needed scene verb via
   the production input path if current verbs cannot express the interaction.
7. **AT-P1-07 — Installed parity:** execute the live VM sequence below with a
   PR-installed build; record each ingress's typed result and board revisions.
   Passing the harness alone does not satisfy this criterion.

### Exit gate: live VM demonstration

**Human e4 → CLI requests Black reply → one host approval → attributed e7e5
on the same board; ungranted MCP/CLI mutation is refused, and a human correction
before a pending commit produces a stale-state result.**

These are future implementation acceptance steps, not commands run for this
spec. Use an isolated VM user/profile and an explicit installed PR channel.
`--ephemeral` skips session restore/save; it is not by itself credential or
profile isolation. Record platform, build SHA, channel, fixture, and actor IDs.
A qualified graphical Linux VM can prove Chess; P5 real playback needs macOS.

Existing command syntax, verified by E20 (replace `NNN` and paths):

```sh
# Run only during implementation validation, on a disposable VM.
just pr-install NNN
plexi-pr-NNN host start --ephemeral --pane 'cwd=/tmp/plexi-agents-proof'
plexi-pr-NNN host status
```

Create the fixture directory before startup. Install/open the reviewed Chess
fixture and open the Assistant in that context using the existing plugin surfaces:

```sh
# Run inside the newly created PR-host terminal, not an ambient beta pane.
plexi-pr-NNN plugin install /absolute/path/to/PLEXI/plugins/chess
plexi-pr-NNN view open chess
plexi-pr-NNN view open assistant
plexi-pr-NNN pane list
plexi-pr-NNN plugin call chess chess.state
```

Fixture setup uses host-authenticated human interaction/consent for install/open
and observation; it must not grant the negative-test callers mutation rights.


1. On the board, the human plays `e2e4`; observe the actual board and record the
   game ID and resulting revision from `chess.state`.
2. Use `assistant send` to submit “For this game, play exactly
   e7e5 as Black once at the current revision.” The deterministic provider
   fixture scripts this tool call. Also run a bounded real-provider variant;
   it must play the requested legal move, not merely describe it.
3. Approve exactly that pending call in the desktop Assistant pane. No second
   seat enrollment occurs. Record the host request/grant IDs.
4. Observe Black's pawn on e5, e7 empty, White to move, one revision increment,
   and the agent's actual identity on the same board and correlated receipt.
5. Use a separate ungranted PR-host terminal actor for MCP/CLI negative calls.
   The token and port below must come from that PR pane's environment. Never
   copy an Assistant's credential or reuse an ambient beta pane credential.
6. For a fresh pending move, make a legal human correction on the board before
   approval. Resume the exact pending request; expect `stale_revision`, no
   agent mutation, and a reread/replan opportunity.

Existing MCP wire syntax (discovery and negative mutation):

```sh
curl --fail-with-body -sS "http://127.0.0.1:${PLEXI_HOST_MCP_PORT}/mcp" \
  -H "Authorization: Bearer ${PLEXI_HOST_MCP_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'

# Replace GAME and REV with the authorized state observation for this fixture.
curl --fail-with-body -sS "http://127.0.0.1:${PLEXI_HOST_MCP_PORT}/mcp" \
  -H "Authorization: Bearer ${PLEXI_HOST_MCP_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chess__chess.play","arguments":{"game_id":"GAME","expected_revision":REV,"operation_id":"negative-mcp","move":"g1f3"}}}'

plexi-pr-NNN plugin call chess chess.play \
  --input '{"game_id":"GAME","expected_revision":REV,"operation_id":"negative-cli","move":"g1f3"}'
```

`GAME` and numeric `REV` are template substitutions, not literal valid fixtures.
The doubled `chess__chess.play` follows `<plugin_id>__<declared_tool>`; confirm it in
`tools/list`. Inspect the MCP tool-result error body even if HTTP is successful.
Expected P1 output is `permission_required` or `permission_denied`, with no move.
Do not approve these negative probes. Discard/expire their pending requests.

Existing targeted CLI intake:

```sh
plexi-pr-NNN assistant send --pane-id ASSISTANT_PANE \
  --text 'For this game, play exactly e7e5 as Black once.' \
  --request-id vm-black-reply --json
plexi-pr-NNN host screenshot --output /tmp/agents-p1.png
plexi-pr-NNN host stop
```

`ASSISTANT_PANE` is the numeric pane from discovery; choose a fresh request ID
for independent fixtures. P1's resumable pending-request observation and approval API
are new: proposed `plexi agent request show ID` / `resolve ID --decision …`
spellings belong to P2. P1 must expose a sanctioned observation seam for its
pending IDs; no existing grant CLI is assumed. Desktop approval is required
for this demo. Review/delete the PNG per Testing; retain the evidence report.

## Drift disposition

This maps every item in alignment's “Where they drift.” It assigns ownership,
not progress state. Deferred work has a reason and is not part of a passed gate.

| Drift | Resolution / phase | Boundary or deferral |
|---|---|---|
| 1 — Split authority across hooks, AgentHost, subscriptions and launch; MCP/CLI bypass | P1 replaces permission decisions with the mandatory monitor in dispatch, including launch/subscription host operations and shared grant store. | Runtime capability checks and plugin legality remain defense in depth, not competing authorizers. |
| 2 — Rich grants ignore resource/duration | P1 matches complete binding and enforces once/session/expiry; replacement and session cache use the same key. | Old broad grants are invalidated unless a signed-off explicit reissue flow replaces that default. |
| 3 — LAN phone vs paired HTTPS relay | P2 fixes durable receipts, addressed intake and real cancellation; P4 displays canonical queue/inbox. | HTTPS, single-use pairing, device revocation and outbound relay remain cloud phase P4 work. Deferred because this spec proves local host behavior and authorizes no deployment. Prototype bearer/LAN is explicitly not equivalent. |
| 4 — Text to first matching pane | P2 versioned envelope addresses opaque agent/conversation with request ID, expiry and target revision. | P1 uses one isolated Assistant only for its existing phone demo; this is not the final routing model. |
| 5 — Channel-specific agents vs neutral skills | P2 moves definitions/team to `.plexi/agents` and `.plexi/team.toml`, adds `/agent new --here`, explicit import and ownership lease. | Credentials/operational channel state are not indiscriminately moved. |
| 6 — Lossy events and volatile timeline | P1 durable correlated authority audit; P2 lossless agent drain, IDs, versioning, recovery; P3 persistent checkpoint references and rollback. | Ordinary UI telemetry may remain best effort. App state remains plugin-owned, not duplicated in the drain. |
| 7 — Pi on ungated MCP | P1 gates Pi because every MCP call reaches the same monitor; P2 upgrades credentials to run bindings. | First-party Pi loop-driver adoption still needs authority-model sign-off and its own backend qualification; transport safety cannot wait for it. |
| 8 — Stale docs | P1 documentation task set below corrects scope, phone auth and “no Railway” wording against E1/E17/E19. | This spec commit changes only its document and Active PRMs index, as requested. No third file is silently added. |
| 9 — Home access vs proposed roots | Authority-default section specifies explicit baseline read grants and per-head full trust; P2 installs them only after approval, P4 displays them. | Ian must sign off on amendments before default widening ships; context/root discovery itself grants nothing. |
| Socket identity caveat (inventory) | P1 host-authenticates peer/session, rejects claimed pane impersonation and removes no-pane `user`. | Local OS-user access to the socket is not proof of human interaction or an agent's assignment. |

P1 documentation work must replace mesh appendix H6's raw-path dispatcher claim
with context-owned `evaluate_reach` (E1), and remove phone README's opening
“no auth/no host connection” claim in favor of its real prototype limits (E17).
Qualify “no Railway” wherever the architectural provenance is reused: website
configuration exists (E19); a phone relay is separate work. These corrections are
deferred to P1 to keep this commit's explicitly requested two-file scope.
Mesh's older “no Assistant CLI” claim also needs replacement with E12's limited
pane-addressed intake. Do not copy the appendix as a current-code inventory.

Alignment's unowned gaps are assigned here: the named Agents API, needs-input,
messages and budgets belong to P2; system/output and multi-lead Desk belong to
P4; text tools and video proof belong to P3/P5. Purchasable workflow manifests
remain deferred because neither purchases nor a workflow DSL is needed to prove
these contracts. Any future manifest declares **requested access**, never grants.

The authority model's remaining command-worker sandbox, environment construction,
secret injection and general-project parity work retain their owning spec and
stints. P1 cannot claim them complete because Chess is gated. Before an adapter
advertises those powers, it must satisfy that owner's confinement/egress tests;
otherwise the gate refuses unsupported actions. Broad reads never enable them.

## Decisions needing Ian's sign-off

The approved architecture is the basis for this spec. The following product
policy decisions remain explicit sign-off gates, not questions that block writing
this document. Retain the narrower existing policy until each amendment is decided.

| Decision | Proposed call / alternative | Applies before |
|---|---|---|
| Existing broad grants (authority model) | Invalidate and ask again. Alternative: a one-time migration prompt issues new exact grants; never silently translate resource-free records. | P1 grant migration |
| Destructive command classes always ask (authority model) | Keep fresh exact approval with no posture/profile override; review classifier scope to avoid treating ordinary reversible work as destructive. | P1 risk policy and later worker qualification |
| Terminal reads lose unprompted access (authority model) | Keep terminal reads secret-adjacent and ask-gated. Alternative: narrowly prove origin-context terminals contain no injected secrets; broad-read default does not settle this. | P1 read classification |
| Out-of-process Pi loop (authority model) | Permit only after gating parity, run credential and cancellation evidence. In-process remains default; Pi's external MCP calls are gated regardless. | First-party Pi backend adoption |
| Broad-read default | Approve the visible global ordinary-home read profile and directory-root read profile, with audit, secret exclusions and explicit egress policy; otherwise keep per-root asking. | P2 default profiles |
| Full-trust profile | Explicit per-head opt-in with declared reversible scope, permanent visible indicator, revoke and irreversible asks; no human impersonation or unbounded shell grant. | P2 profile issuance / P4 presentation |
| Phone approval of irreversible actions | Initial policy requires desktop confirmation. Alternative: a paired, revocable device principal may approve the exact action after remote auth qualification. LAN shared bearer is insufficient. | Any remote approval rollout |
| Channel-neutral agents move | Approve neutral workspace/user definitions with stable IDs, explicit collision handling, single supervisor ownership and no copied credentials. | P2 one-way migration |

A signed-off bounded assignment extends exact-argument consent deliberately:
the user approves its resource/action envelope once, then each concrete commit
is resolved, checked and fingerprinted within that envelope. Assignment scope
never authorizes irreversible effects without their own exact request. The
scope UI must show this distinction; a bare “allow tool” checkbox cannot express it.
An assignment/profile grant binds its approved envelope and policy version;
the monitor proves containment and creates an exact per-call authorization binding.
It does not implement scoped authority by omitting resource or argument fields
from the P1 exact-call matcher. Both envelope and concrete use are auditable.

## Verification method and evidence contract

Follow [TESTING.md](../../src/testing/TESTING.md), with these phase-specific
applications. The document-writing change does not run builds/tests or claim
any acceptance criterion passed.

| Layer | Required evidence during implementation |
|---|---|
| Rust invariants | Grant binding, normalization, revocation, schema validation, allocation arithmetic, idempotency, journal recovery, editor/video transaction and projection tests. |
| HostHarness | Production command/tool ingress, real plugin runtime, scripted Assistant loop, correlated replies/events and hidden/inactive/no-view service paths. |
| TOML scenes | Observable board, editor review, request/conversation/Desk and video timeline behavior; seeded realistic content and semantic assertions. |
| Installed host | Isolated profile/VM, explicit PR channel and build provenance, actual CLI/socket/MCP/phone boundaries, restart and conditional mutation evidence. |
| Visual review | Identify scene/screenshot surface before UI implementation; render, open and inspect PNG, delete it, record what was reviewed. Sanctioned `plexi host screenshot` only for live capture. |
| Live model / phone | Bounded provider behavior and physical device checks reported separately from scripted model/browser emulation. Never substitute a mock for missing live evidence. |

Use explicit fixture clocks, deadlines, queue sizes, retry bounds and budgets.
Use readiness/event barriers and bounded eventual assertions, not workflow sleeps.
Use `load_aware_timeout` for real process waits according to Testing. Worker
threads must receive explicit test-profile paths; thread-local harness isolation
must not accidentally write audit/journal data to the user's actual profile.
Inject failures only through test-only seams, never a public authorization bypass.
Prove load-bearing tests detect a controlled broken invariant before accepting them.

Every phase report identifies build SHA, platform, channel/profile ownership,
fixture, provider mode, test/scene IDs, principal/agent/run/request IDs, grant and
operation IDs, revisions, actual outcomes, restart/recovery and teardown result.
Redact credentials and private content. Unknowns and unsupported live operations
are named failures for required criteria, not silent skips or claimed passes.
A phone timeout is not a Chess success; an exit-zero worker is not a typed result.

The implementation's pre-push build/test/lint sequence and editor-specific release
gate remain owned by Testing; this spec does not duplicate or relax them. No
phase is complete merely because a lower-layer test passes. App/host state is
observed through CLI/SDK tools and events, never private profile-file inspection.
Each new capability emits an info-level boundary trace as root AGENTS requires.

## Out of scope

Payments, checkout and purchasable workflow delivery; Railway phone-relay deploy;
merging to alpha or cutting releases; hosted runtime and multi-tenant qualification;
full no-window daemon qualification; arbitrary native sandbox guarantees; general
workflow DSL; universal merge/CRDT and destructive-media selective undo; full video
export/NLE. Local P1–P5 contracts must remain usable without any of those services.

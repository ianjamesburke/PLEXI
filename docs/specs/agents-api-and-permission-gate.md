Status: active
Stint: none yet

# Agents API and one permission gate (P1–P5)

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
| Architecture package: journeys (Grok Bot, 2026-10-05; not in repo) | Directory lead, unconfigured folder, app launch, phone approval, creative editing, and multi-lead shell. |
| Architecture package: proof apps (Grok Bot, 2026-10-05; not in repo) | Chess before creative editing; this document's P1–P5 order governs these proofs. |
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
| [Agent mesh](../assistant-agent-mesh.md) | Retains heads, typed recall/drain, bounded `ask_question`, §6 migration, and §8 one conversation per head. P2 supersedes root-path identity with opaque IDs; paths become lookup views. |
| [Run orchestration](../agent-run-orchestration.md) | Retains observed liveness, leases, bounded supervision, and reconcile-on-restart. P2 supersedes names-as-identity and independent authoritative per-run journals for agent work. |
| [Host Assistant](../assistant-host-app.md) | Retains tools, content blocks, permission UX, settings and backend separation. P1 replaces optional permission hooks; P2 replaces pane-owned intake and channel-specific agent definitions. |
| [XML video editor](../xml-video-editor.md) | Owns XML dialect, timing, round-trip and playback/export fidelity. P5 adds ChangeSets; it does not replace XML with a private EDL database. |
| [Testing](../../src/testing/TESTING.md) | Owns implementation build gates, Rust/scene split, isolation, editor gate, and mandatory visual review. |

Host Assistant history actions that fork or switch threads do not create another
canonical head conversation: mesh §8 supersedes that older UI. Exported history
and child-run transcripts remain separate artifacts linked to the head.
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
| ChangeSet | Prepared delta against an app-owned resource, with revisions, footprint, preview, selected hunks and attributed commit/revert receipts. |
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
| E2 | `src/plexi_ai/tool_dispatch.rs:551` `set_hooks`; `:594` `dispatch_call`; `:644` `dispatch_inner` | Hooks are optional on host and app tool calls. Inner dispatch sends the caller string; a snapshot is not a live grant check. |
| E3 | `src/broker/mod.rs:133` `GrantRecord`; `:183` `matches`; `:208` `PermissionRequest`; `:435` `record` | Resource fields exist on grants but are absent from requests/matching/replacement identity. Expiry is checked; duration itself is not enforced by matching. |
| E4 | `src/assistant/mod.rs:149` `AssistantToolHooks`; `:963` `gated_dispatcher`; `:1039` `set_hooks` | Assistant installs the ask hook, keeps session approval by tool name, and auto-allows app-asserted read-only tools under Ask. |
| E5 | `src/agent/mod.rs:740` `gated_dispatcher` | AgentHost filters a turn snapshot from broker decisions and does not install the Assistant hook. |
| E6 | `src/app/host_mcp.rs:73` `register_pane_credential`; `:349` dispatcher construction; `:385` tool routing; `:393` `dispatch_call` | MCP bearer binds a pane/context/root. App tools dispatch without a grant hook; event tools have separate service routing. |
| E7 | `src/app/app_call.rs:19` `caller_identity`; `:40` context lookup; `:56` dispatcher; `src/app/lifecycle.rs:2305` `CallAppTool`; `src/app/mod.rs:583` `handle_socket_line` | App-call accepts a forwarded pane ID; existence is checked, peer ownership is not established on this path. No pane maps to `user`. Notification peer stamping does not fix app-call. |
| E8 | `apps/chess/chess.py:170` `_tool_play`; `apps/chess/chess_domain.py:137` `play`; `:158` receipt lookup; `:283` `_authorize` | Host caller string reaches Chess. Seat checks are separate; revision, operation ID, immutable next-game value and receipt/outbox already exist. |
| E9 | `src/assistant/audit.rs:11` `AuditEvent`; `:27` `now` | Audit schema lacks call/grant IDs and stamps the literal `agent:assistant`. |
| E10 | `src/agent/mod.rs:281` `AgentRegistry::load`; `:346` `workspace_agents_dir`; `src/assistant/skills.rs:65` `SkillRegistry::load` | Agent tiers are builtin/user/workspace, with later IDs shadowing earlier ones. Workspace agents use the channel dir; workspace skills use literal `.plexi/skills`. |
| E11 | `src/assistant/mod.rs:2414` agent creation; `src/app/registry.rs:949` `resolve_workspace_root_with_channel` | Agent creation writes the profile tier. Workspace resolution seeks the channel marker and excludes home before checking it. |
| E12 | `src/app/lifecycle.rs:219` `SubmitAssistantTurn`; `src/cli/app.rs:1932` `assistant_send_cli` | Text and request ID go to the first matching Assistant pane, optionally filtered by pane/context. No durable agent/conversation addressing here. |
| E13 | `src/host/event_log.rs:278` `emit_scoped`; `src/host/app_timeline.rs:285` `AppTimeline`; `:638` `begin_rollback`; `:678` `resolve_rollback_verify` | Event writer drops on full. Timeline is in memory; rollback entrypoints are compiled only for tests. |
| E14 | `src/editor/transaction.rs:72` `Transaction`; `src/editor/history.rs:24` `EditHistory`; `src/app/text_editor_app.rs:1000` `impl App` | Grouped editor transactions exist without actor/run/op tags. The inspected builtin App implementation has no `ExposeTools` or `ToolCall` handling. |
| E15 | `src/app/assistant_host_tools.rs:465` file edit routing; `:1229` `edit_scoped_file` | Assistant editing reaches a direct filesystem string replacement/write, outside editor history and without expected revision. |
| E16 | `src/media/video.rs:119` `VideoDecoder`; `:212` non-macOS implementation; `:694` `MockVideoDecoder`; `src/app/video_player_app.rs:27` `new` | Existing player uses the decoder service. Production non-macOS open returns `NotImplemented`; a procedural mock exists for tests. |
| E17 | `clients/phone-web/server.py:232` `HostStore`; `:253` CLI call; `:216` cancel; `:404` flags; `:412` token policy | Host mode invokes Assistant send; receipts are in memory, cancel changes phone state only, and host mode requires a bearer even on loopback. |
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
| Global head | Audited broad reads of ordinary home files and permitted cross-context summaries, represented by a visible baseline read grant. | Secret access, terminal scrollback, writes, sends, process execution, app control, or blanket export of home content to a model. |
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

Host classification, not an app's `read_only` claim, decides risk. Unknown tools
ask. Broad read excludes credential stores, known secret files and terminal
output by default; a redactor is a backstop, not proof arbitrary files contain
no secrets. Read authorization and remote model transmission are distinct:
provider-bound content is minimized and subject to the configured egress grant.
The profile UI makes this distinction visible.

**Runtime limit:** host checks constrain host-mediated calls. They cannot
constrain a CLI agent that disables/bypasses its own sandbox and reads or writes
through native OS syscalls, nor prove that a malicious app's claimed footprint
is honest. A cwd or git worktree is not OS isolation. Untrusted code/workflows
run in a qualified VM/sandbox with explicit mounts, network and secrets policy.
P2 may supervise trusted CLI work but must not advertise a native isolation
boundary it has not qualified. The authority model retains ownership of the
command-worker sandbox and secret-injection implementation.

## P1 — One permission gate, resource-scoped grants, Chess

### Goal, scope, non-goals

Prove one human approval authorizes one exact attributed move, independent of
which ingress proposes it. Close optional-hook and socket identity bypasses.

Scope includes app and host tools, Assistant and AgentHost, host MCP, socket
app-call and CLI; resource/argument-bound grants; one pending permission service;
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
   `plexi needs-you list` reads those rows. A desktop click calls
   `approve_pending`. The terminal cannot resolve them. The phone
   `/api/needs-you` route reads the same rows.
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
log. Outcomes after a crash can be reconciled from app receipts, or explicitly
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
   parameterized by ingress and adversarial case, loads `apps/chess` through its
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
   app-call, native host tools, app launch and event subscription. Remove a grant
   after discovery; invocation still refuses. An app claiming `read_only=true`
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

**Human e4 → phone requests Black reply → one host approval → attributed e7e5
on the same board; ungranted MCP/CLI mutation is refused, and a human correction
before a pending commit produces a stale-state result.**

These are future implementation acceptance steps, not commands run for this
spec. Use an isolated VM user/profile and an explicit installed PR channel.
`--ephemeral` skips session restore/save; it is not by itself credential or
profile isolation. Record platform, build SHA, channel, fixture, and actor IDs.
A qualified graphical Linux VM can prove Chess; P5 real playback needs macOS.

Existing command syntax, verified by E20 and E17 (replace `NNN` and paths):

```sh
# Run only during implementation validation, on a disposable VM.
just pr-install NNN
plexi-pr-NNN host start --ephemeral --pane 'cwd=/tmp/plexi-agents-proof'
plexi-pr-NNN host status
```

Create the fixture directory before startup. Install/open the reviewed Chess
fixture and open the Assistant in that context using the existing app surfaces:

```sh
# Run inside the newly created PR-host terminal, not an ambient beta pane.
plexi-pr-NNN app install /absolute/path/to/PLEXI/apps/chess
plexi-pr-NNN app open chess
plexi-pr-NNN app open assistant
plexi-pr-NNN pane list
plexi-pr-NNN app call chess chess.state
uv run --python 3.11 clients/phone-web/server.py \
  --backend host --plexi-bin plexi-pr-NNN --lan --port 8788
```

Fixture setup uses host-authenticated human interaction/consent for install/open
and observation; it must not grant the negative-test callers mutation rights.
The phone server's printed LAN URL supplies its prototype bearer. The URL is
sensitive fixture material and must be redacted from evidence. The current
server has no pane-selector flag; isolate a single Assistant in this P1 fixture.
P2 replaces that constraint with agent/conversation addressing.

1. On the board, the human plays `e2e4`; observe the actual board and record the
   game ID and resulting revision from `chess.state`.
2. Open the phone page with `--backend host`; submit “For this game, play exactly
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

plexi-pr-NNN app call chess chess.play \
  --input '{"game_id":"GAME","expected_revision":REV,"operation_id":"negative-cli","move":"g1f3"}'
```

`GAME` and numeric `REV` are template substitutions, not literal valid fixtures.
The doubled `chess__chess.play` follows `<app_id>__<declared_tool>`; confirm it in
`tools/list`. Inspect the MCP tool-result error body even if HTTP is successful.
Expected P1 output is `permission_required` or `permission_denied`, with no move.
Do not approve these negative probes. Discard/expire their pending requests.

Existing targeted CLI intake can diagnose the phone path independently:

```sh
plexi-pr-NNN assistant send --pane-id ASSISTANT_PANE \
  --text 'For this game, play exactly e7e5 as Black once.' \
  --request-id vm-black-reply --json
plexi-pr-NNN host screenshot --output /tmp/agents-p1.png
plexi-pr-NNN host stop
```

`ASSISTANT_PANE` is the numeric pane from discovery; choose a fresh request ID
for independent fixtures. Do not submit both phone and CLI turns as if they
were one operation. P1's resumable pending-request observation and approval API
are new: proposed `plexi agent request show ID` / `resolve ID --decision …`
spellings belong to P2. P1 must expose a sanctioned observation seam for its
pending IDs; no existing grant CLI is assumed. Desktop approval is required
for this demo. Review/delete the PNG per Testing; retain the evidence report.

## P2 — Agents API core

### Goal, scope, non-goals

Make heads and their work durable, addressable and bounded independently of
panes. A temporary child can finish for its parent without acquiring new rights
or multiplying the parent's budget. Closing a view does not spawn or kill work
unless the explicit run policy says so.

Scope: opaque identities, portable definitions, teams, assignments, delegation,
run tokens, receipts, typed requests, lossless agent drain, canonical conversation
ownership, CLI/MCP adapters and local phone intake migration.

Non-goals: Desk UI, universal workflow DSL, autonomous permanent roster changes,
cloud/daemon runtime qualification, remote pairing, public tenancy, a new A2A
transport, or billing based on inferred terminal activity.

### Proposed records and operations

```rust
struct Agent {
    id: AgentId, name: String, definition: DefinitionRef,
    role: AgentRole, team_id: Option<TeamId>,
    canonical_conversation_id: Option<ConversationId>, // required for heads
}
struct Team {
    id: TeamId, context: ContextBinding, root: RootRef,
    head: AgentId, members: Vec<AgentId>, policy: PolicyRef,
}
struct Assignment {
    id: AssignmentId, owner: AgentId, requested_by: PrincipalId,
    parent: Option<AssignmentId>, goal: String, scope: Scope,
    budget: Budget, deadline: Timestamp, kind: WorkKind,
    pays_back: Option<PaybackIntent>, review_on: Option<Date>,
}
enum WorkKind { System, Output }
struct Delegation {
    id: DelegationId, parent_run: RunId, child: AgentId,
    assignment: AssignmentId, requested_scope: Scope,
    effective_scope: Scope, reservation: ReservationId,
    expires_at: Timestamp, template: TemplateVersion,
}
struct Run {
    id: RunId, agent: AgentId, assignment: AssignmentId,
    delegation: Option<DelegationId>, parent_run: Option<RunId>,
    config_snapshot: ConfigRef, generation: u64, deadline: Timestamp,
    observation: LivenessObservation, claimed_state: Option<Claim>,
    reservation: ReservationId, result: Option<TypedResultRef>,
}
struct RunTokenBinding {
    principal: PrincipalId, agent: AgentId, run: RunId,
    assignment: AssignmentId, delegation: Option<DelegationId>,
    host_id: HostId, generation: u64, expires_at: Timestamp,
}
```

All IDs are opaque and host-issued. Names, root paths, panes, provider sessions,
team roles and thread labels are views/foreign keys, never identity aliases that
silently retarget work. A rename preserves ID, conversation and grants. Moving a
root requires explicit rebinding and revalidation of resource grants.
The global head has an explicit storage identity, never rootless-context fallback.

| API operation | Contract | MCP tool adapter / proposed CLI |
|---|---|---|
| `agents.list/get`, `teams.list/get` | Authorized identity/card queries; no secrets | `plexi__agents_list`; `plexi agent list/show`, `plexi agent team show` |
| `assign` | Goal + owner + scope + budget + kind; persist before receipt | `plexi__assign`; `plexi agent assign --input FILE` |
| `delegate` | Parent allocation + allowed template + child scope | `plexi__delegate`; `plexi agent delegate --input FILE` |
| `send` | Versioned intake/message + receipt; no authority in prose | `plexi__send`; `plexi agent send --input FILE` |
| `request_input` | Schema-valid question/options/trade-off/recommendation/time cost | `plexi__request_input`; `plexi agent request create --input FILE` |
| `resolve` | Address exact typed request; verify resolver authority/version | `plexi__resolve`; `plexi agent request resolve ID --input FILE` |
| `runs.observe/cancel` | Claim and observation together; revoke and settle on cancel | `plexi__runs_observe`; `plexi agent run show/cancel ID` |
| `events.subscribe/recall` | Authorized filter + cursor, bounded replay/typed recall | `plexi__events_subscribe`; `plexi agent events --after CURSOR` |
| `propose_change/commit/revert` | P3 resource adapter operations | `plexi__propose_change`; `plexi agent change …` |

These spellings are proposals, not commands available at E20. The existing
orchestration proposal `plexi run` is superseded for agent work by
`plexi agent run`; do not implement aliases or two schedulers. Existing unrelated
agent hook/report utilities retain their distinct documented purposes.

### Delegation and resource admission

Effective child authority is the intersection of user grants, parent **delegable**
authority, requested child scope, pinned template restrictions and resource
policy, as in cloud §4.4. Intersect resources, verbs, secrets, network destinations,
expiry, model policy, spawn depth, retry/turn limits and budget. Empty intersection
is refusal. A parent's broad-read baseline is not automatically delegable.
A child cannot rewrite settings, prompt text or team membership to widen rights.

Reserve budget and a concurrency/resource lease atomically before starting work.
Children spend from the same parent allocation tree; retrying admission reuses
its reservation. Race two requests against one remaining slot: at most one wins.
Record measured usage, estimates, outstanding reservations and settlement
separately. Unknown provider costs remain reserved/conservative; do not claim a
hard monetary cap for unbounded native commands or unknown-cost operations.

Runs pin configuration and have deadlines, model/turn/descendant limits, bounded
output and process cleanup. Successful process exit without the requested typed
result is not successful work. Host observations carry timestamps and source:
process alive, last output, heartbeat, phase deadline and terminal evidence.
A claim of `running` cannot keep a dead worker green.

Run tokens replace pane bearers for agent execution. Authenticate them at MCP
and the other adapters, then consult live scope and revocation at P1's gate.
Revoke at run end, cancellation, parent cancellation, expiry or lost ownership;
rotate on recovery. A pane only displays a run and cannot extend its lifetime.
External human sessions may have separate device/session tokens, never fabricated
agent runs or credentials inherited from another pane.

On restart, a single-writer lease and generation fence reconcile process identity,
provider session and durable receipts before resuming. Never spawn a replacement
merely because an ACK was lost. If the old worker cannot be fenced/reconciled,
report `unknown` and block duplicate side effects until recovery. Late child
results remain attributable but cannot resurrect cancelled authority.

### Intake, messages and typed requests

Use the cloud §4.2 envelope across pane, CLI, phone and test client:

```json
{
  "schema_version": 1,
  "request_id": "req_opaque",
  "host_id": "host_opaque",
  "agent_id": "agent_opaque",
  "conversation_id": "conversation_opaque",
  "expires_at": "2030-01-01T00:01:00Z",
  "content": [{"type": "text", "text": "Analyze this game without moving."}],
  "target": {"resource_id": "game_opaque", "expected_revision": 1}
}
```

The timestamp is a controlled-clock fixture. Server authentication supplies the
sender/authority; payload IDs are requests to resolve, not proof of access.
`plexi assistant send --input FILE` replaces pane selection and routes through
this envelope; an optional text convenience must resolve a selected opaque head
and fill the same fields before submission. No “first matching pane” fallback.
The parser rejects unsupported versions, oversized input, expired requests and
agent/conversation mismatch. Dedup key is authenticated owner plus request ID:
same payload returns the existing receipt; changed payload conflicts.

```rust
enum ReceiptState {
    Queued, Delivered, Accepted, Running,
    Waiting { request: RequestId },
    Terminal { outcome: Outcome, result: Option<ResultRef> },
    Unknown { reason: String },
}
struct Message {
    id: MessageId, sender: PrincipalId, recipient: AgentId,
    conversation: ConversationId, speech_act: SpeechAct,
    request_id: RequestId, parent: Option<MessageId>, hop_limit: u16,
}
enum Request {
    NeedsInput { id: RequestId, version: u64, question: String,
        options: Vec<ExplainedOption>, trade_off: String,
        recommendation: String, time_cost: Duration, deadline: Timestamp },
    Permission { id: RequestId, version: u64, pending_call: CallId,
        exact_binding: GrantBinding, deadline: Timestamp },
}
```

Queued means durably received; delivered means routed to the recipient inbox;
accepted means admitted by its owner; running requires host observation.
Terminal distinguishes succeeded/failed/cancelled/expired; unknown is visibly
unreconciled, never success. Future edge acceptance/delivery states remain
transport metadata and cannot masquerade as host acceptance.

One FIFO queue and one active model turn per head join desktop, phone, routines,
peers and events. Different heads run concurrently within leases and budgets.
Permission replies and cancellation address active runs directly. Bound queues,
trigger rates and hop counts; refuse routing cycles. `ask_question` remains the
mesh's informational query with deadline, refusal, ambiguity and answer records;
it cannot create a delegation or make a peer act with stronger authority.

`needs_input` resolution supplies information. `permission` resolution invokes
P1's exact pending-call decision. Discuss/acknowledge/snooze never approve.
Permission requests retain their exact fingerprint and expiry across restart;
replayed approvals are idempotent, version-mismatched decisions fail visibly.
The host owns a single conversation writer; peer receipts project inline from
stable event/message IDs, not independently appended transcript files.

### Drain and storage

```rust
struct AgentEvent {
    schema_version: u32, record_id: RecordId, sequence: u64,
    timestamp: Timestamp, host_id: HostId, actor: PrincipalId,
    agent_id: Option<AgentId>, request_id: Option<RequestId>,
    run_id: Option<RunId>, assignment_id: Option<AssignmentId>,
    call_id: Option<CallId>, grant_id: Option<GrantId>,
    operation_id: Option<OperationId>, resource: Option<ResourceVersion>,
    caused_by: Option<RecordId>, parent_id: Option<RecordId>,
    event_type: String, payload: VersionedPayload,
}
```

Agent records use a backpressured, durable acknowledged append, never the
`try_send` drop path in E13. Persist before acceptance or side-effect intent;
flush before clean shutdown. Disk-full/torn-record/corrupt-format conditions are
visible errors; admission pauses rather than report fictitious persistence.
Keep the UI thread responsive while the journal worker waits. Unknown event
variants preserve their payload and cannot drive unsafe state transitions.

Use an append-only versioned JSONL journal with rebuildable indexes and portable
snapshots, one authoritative writer per host domain. Per-run directories retain
pinned config and artifacts; their event views are projections/pointers into the
drain, not another writable source. This supersedes orchestration's independent
per-run event source for Agents API work. Audit projections keep their retention
independent of conversation deletion but reference the same records.

Proposed storage root: `~/.plexi/agent-records/<host-id>/`, owned by the host,
with a channel-independent writer lease and explicit export/import for portable
teams. Canonical conversations and receipts survive a channel handoff there;
credentials and process handles do not. This amends Host Assistant's blanket
channel-aware storage rule for Agents API records only. A second channel must
attach through the owning host or request a handoff, never open another writer.

App state stays app-owned. Commit state, operation receipt and outbox together;
forward to the existing bus/drain with idempotent record IDs. Acknowledged drain
receipt advances the outbox/consumer cursor. At-least-once delivery plus dedup is
the guarantee, not exactly-once arbitrary external effects. A crash between a
native send and its receipt can remain `outcome_unknown`.

Subscribers filter by authorized actor/context/resource and resume from a cursor.
Retention must preserve live receipt/recall references or an explicit archive
pointer. An expired cursor yields `resync_required` plus an authorized snapshot;
no silent gap. Non-agent UI telemetry may remain best effort, visibly classified.
A CLI drain write reaches the host or fails; it cannot silently emit into a
process-local uninitialized logger.

### Agent definition migration

Workspace definitions move to `<root>/.plexi/agents/<definition>/` with `AGENT.md`
and `settings.toml`; `<root>/.plexi/team.toml` references opaque agent IDs and the
head/member roles. User definitions move to `~/.plexi/agents/`; installed package
binaries, credentials and operational channel profiles remain channel-specific.
Workspace skills already use `.plexi/skills` (E10); this does not assert that all
skill tiers or all other workspace data are channel-neutral.

```toml
# Proposed team.toml; membership requests no implicit permission.
schema_version = 1
id = "team_opaque"
head = "agent_lead_opaque"
members = ["agent_script_opaque"]
# Root identity is host-bound from the containing workspace, not trusted text.
```

Separate definition keys from durable instances. Precedence may select a prompt
version, but a shadowing file cannot replace an instance's identity or grants.
`/agent new --here` creates the workspace definition and proposed team member
through an authorized write; it does not silently create a global agent.
First creation before P3 uses an exact reviewed file-write operation; P3 routes
it through ChangeSet preparation. Root discovery for teams recognizes neutral
`.plexi` independently of channel app discovery and still excludes home as an
accidental workspace. No unrelated app-loader rewrite is implied.

Migration is explicit, transactional and one-way: enumerate legacy definitions,
preview collisions, assign stable IDs, select the authoritative definition,
write the neutral tier and an import receipt, then disable old-tier loading.
Divergent definitions require a choice; a retry of the migration cannot mint
new identities. Do not dual-read, mirror or maintain old directory aliases.
Grants are reissued/rebound only through consent, never copied by display name.
Channel switching preserves team identity but does not import another channel's
live credentials or start a duplicate team supervisor; use a workspace ownership
lease and an explicit handoff. Revoked pane-only tokens stop authenticating.

### Acceptance criteria

1. **AT-P2-01 — Narrow delegation (E2E-B shape):** HostHarness test
   `parent_child_analysis_cannot_escalate` uses scripted parent/child model loops
   to analyze one Chess game without moving. Child attempts another game, a
   write, a secret, spawn and larger budget; each is denied before effect.
   Close the parent view, deliver the typed analysis, and verify unchanged board,
   one parent receipt and settled reservations. A bounded live-provider VM run
   repeats the same task and records its actual model behavior separately.
2. **AT-P2-02 — Budget and restart:** Rust tests
   `delegation_reservations_are_atomic` and `restart_reconciles_worker_generation`
   race admission against one slot, crash before/after ACK, cancel the parent,
   and deliver a late result. Verify one worker/result, no multiplied budget,
   token revocation and no unauthorized late mutation. Repeat process restart
   in the installed VM because an in-memory fold cannot prove OS cleanup.
3. **AT-P2-03 — Identity and migration:** Rust test
   `agent_rename_and_channel_migration_preserve_identity` renames a head, switches
   channel readers, repeats migration, injects conflicting definitions and
   verifies stable IDs/conversation, explicit collision failure and no dual writer.
4. **AT-P2-04 — Dropped-event regression:** Rust test
   `agent_drain_backpressure_never_drops_acknowledged_record` fills the queue with
   a paused writer, resumes it, restarts and reads every acknowledged record ID.
   Inject full disk, torn tail and clean shutdown with pending records; failures
   are surfaced and no successful acceptance disappears. Demonstrate the test
   fails when the old drop-on-full implementation is substituted.
5. **AT-P2-05 — Intake and token binding:** HostHarness table test
   `intake_adapter_contract` sends one envelope through Assistant, CLI/socket,
   MCP and phone adapter parsers. Duplicate input reuses a receipt; changed
   payload, expired request, foreign conversation and ended-run token fail.
   Verify hidden-window and inactive-context servicing through production drains.
6. **AT-P2-06 — Requests are not consent:** Rust test
   `request_resolution_is_typed_and_versioned` exercises discuss, snooze,
   needs-input answer, exact permission approval, expiry and replay. Only the
   authorized permission decision can resume its exact pending operation.
   TOML scene `agents-request-receipts.toml` proves distinct labels and inline
   attributed peer receipts in a canonical conversation.
7. **AT-P2-07 — Installed drain recovery:** VM step disconnects the phone,
   restarts the host with a child pending, reconnects from an old cursor and
   opens the same head in desktop/phone views. Verify one conversation, no
   duplicate child, visible unknown/recovery state where needed and consistent
   durable receipt history; an expired cursor explicitly requests resync.

### Exit gate and migration completion

On the isolated PR-installed VM, perform AT-P2-01, AT-P2-02 and AT-P2-07 through
public adapters. Capture assignment/delegation/run IDs, effective scope,
reservations, lifecycle observations, receipt/event cursors and revoke results.
Exercise no-view execution inside the still-running host, a hidden window and an
inactive context independently. A full no-window daemon is not required here.
Demonstrate rootless onboarding, `/agent new --here`, rename and restart without
worker duplication. Phone host mode must now use the durable envelope/receipts;
its in-memory receipt authority and local-only cancel behavior are removed.
Cancellation reaches the run and stops future work; committed effects stay in
receipts. Missing live observation commands must be implemented before this gate
can pass. No private state-file reading substitutes for public API evidence.

## P3 — ChangeSets on the text editor

### Goal, scope, non-goals

Let an agent propose edits in the same document a person is editing. Review and
permission are separate: permission grants the right to propose/commit within a
scope; creative acceptance chooses which prepared hunks should become real.

Scope: builtin Rust editor tool registration, prepare/preview/commit/revert,
revision checks, per-hunk review, transaction attribution, selective compensating
undo and production timeline rollback wiring. Existing editor internals are E14;
direct file writes and test-only rollback seams are E15 and E13.

Non-goals: universal CRDT, arbitrary multi-file atomicity, reverting external
terminal writes, or claiming a whole-document snapshot is selective undo.

### Proposed shape and operations

```rust
struct ChangeSet {
    id: ChangeSetId, resource: ResourceId, generation: u64,
    base_revision: Revision, actor: AgentId, run: RunId,
    assignment: AssignmentId, operation_id: OperationId,
    hunks: Vec<Hunk>, footprint: ReadWriteFootprint,
    prepared_digest: Digest, preview: PreviewRef,
}
struct Hunk {
    id: HunkId, depends_on: Vec<HunkId>, preconditions: Preconditions,
    edits: Vec<TextEdit>, inverse: Option<Vec<TextEdit>>,
}
struct AttributedTransaction {
    transaction: Transaction, actor: PrincipalId, run: Option<RunId>,
    operation_id: OperationId, change_set: ChangeSetId,
    selected_hunks: Vec<HunkId>, before: Revision, after: Revision,
}
```

| Operation | Contract |
|---|---|
| `editor.read(resource, range)` | Return canonical buffer text, identity, revision and dirty state under a read grant; bounded range, no disk-shadow read. |
| `editor.propose_edit(resource, expected_revision, operation_id, edits)` | Prepare immutable hunks/footprint and preview; no canonical mutation. |
| `changes.preview(id)` | Render diff, attribution, affected ranges and dependent hunks from the prepared value. |
| `changes.commit(id, selected_hunks, expected_revision)` | Bind selected payload/digest, recheck scope and revision, apply one grouped transaction, return receipt. |
| `changes.revert(id, expected_revision)` | Prepare an attributed inverse/compensation; commit only if independent and approved by the configured review policy. |

Expose the editor through a **Rust-side registration** in the tool registry,
with a builtin handler and host-owned resource identity. There is no Python SDK
inside `TextEditorApp`; importing `ExposeTools` is not an implementation plan.
MCP's wire name may namespace a declared tool, but `editor.read` and
`editor.propose_edit` are stable logical tool IDs.

A document service owns the buffer/revision across multiple editor views.
Resolve paths to that service before authorizing. `host.files.edit` targeting an
open document prepares a ChangeSet there, including unsaved human text; it must
not write its on-disk version behind the editor's back. Apply the same routing to
host file-write variants that could bypass it. Closed-file edits use the same
revision/precondition contract with an exact file handle and atomic write.
Saving a buffer is a separately authorized disk action when needed.

Use explicit character/grapheme-safe coordinates and expected old text/hashes;
reject byte-offset ambiguity and invalid boundaries. Preflight all operations
on a copy before committing atomically; a later bad hunk cannot leave earlier
ones applied. Break typing coalescence around the agent group so human edits
never share its undo unit. Partial acceptance is one selected, dependency-closed
set of hunks; dependent selections either expand visibly or fail, never apply
hidden edits. Selection changes generate a new concrete commit digest.

Initially a whole-document revision is a safe conflict boundary. More precise
object/range versions may allow independent commits only after they have tests.
Concurrent human edits invalidate pending overlapping work; return `edit_conflict`
with authorized current revision and a fresh preview. The old grant is not
rebound silently to the new proposal.

Selective revert checks intervening operations and dependencies, not just text
equality. Only independent agent edits may be inverted automatically. If human
work changed/depended on them, preserve the current buffer and create a
compensating preview for review. Never restore an old whole-file snapshot over
human corrections. If historical dependency information was pruned, report
unsupported/conflict rather than guess. A successful revert is itself an
attributed operation and receipt, with no deletion of the original audit.

Wire `AppTimeline::begin_rollback` / verification into production ChangeSet
revert and the public UI/CLI command. Verification alone is not an apply path:
follow it with conditional adapter commit, receipt and correlated event. Store
checkpoint references durably in the P2 drain; rebuild the in-memory timeline.
Conversation rewind only selects history unless the user explicitly requests
this supported, previewed state change.

### Migration

Replace direct host file mutation for open buffers and the test-only rollback
path outright. Existing transactions/history gain attribution and operation IDs;
legacy human history may remain labeled human/unknown provenance, never invented
agent authorship. Do not run a disk patcher beside the buffer service. Reject
unsupported old tool schemas rather than keep a second mutation protocol.
This extends Host Assistant's checkpoint contract; it does not promise rollback
for terminal commands or adapters without independent inverses.

### Acceptance criteria

1. **AT-P3-01 — Revision-bound grouped commit:** Rust test
   `editor_changeset_commit_is_atomic_and_idempotent` covers Unicode edits,
   invalid later hunk, selected-hunk digest changes, stale revision and duplicate
   operation. Verify one attributed undo group or no mutation.
2. **AT-P3-02 — Selective revert preserves human work:** Rust table test
   `editor_revert_preserves_intervening_human_edits` covers disjoint edits,
   overlap, dependency and pruned history. Only independent inverses commit;
   all ambiguous cases return a compensating preview without changing the buffer.
3. **AT-P3-03 — No disk bypass:** HostHarness test
   `host_file_edit_routes_to_dirty_editor` opens an unsaved document, dispatches
   `host.files.edit` through each adapter and checks the same prepared ChangeSet,
   buffer, revision and audit. A denied/stale edit changes neither buffer nor disk.
4. **AT-P3-04 — Inline review and rollback:** TOML scene
   `editor-agent-hunks.toml` presents realistic human/agent text, accepts/rejects
   individual hunks by keyboard, shows actor/run credit, edits as the human, and
   invokes timeline revert. Semantic assertions and reviewed screenshots prove
   inline preview, conflict preservation and result visibility.
5. **AT-P3-05 — Installed co-editing:** VM step opens one real file, proposes an
   edit from a run, accepts a subset, types a human correction, then requests
   agent-only revert. Verify the correction survives save/reopen and a conflicting
   inverse remains a preview. Run the editor release gate required by Testing
   during implementation and record its evidence separately.

### Exit gate

AT-P3-05 must pass on an isolated PR-installed host. Observe changes through the
public editor tools and pane semantics, inspect the actual diff/inline review
pixels, and correlate transaction, run, operation and grant IDs. Reopen after
host restart to prove checkpoint/receipt continuity. Preserve source fixtures and
report; review/delete generated screenshots per Testing. A disk diff alone is
not evidence of editor transaction integration.

## P4 — Multi-lead command view merged with Desk

### Goal, scope, non-goals

Build **one Desk shell** over the Agents API: the Plexi team plan's All work /
per-context view, with lead lanes, headless queue and Needs you. It is the primary
multi-lead command view, not another task database or a competing “Command” app.

Scope: tasks/assignments, activity summary, agent conversations, direct member
selection, workspace/terminal links, ranked typed inbox, observed liveness,
System | Output accounting and the same mobile projection.

Non-goals: a god-agent routing bottleneck, autonomous policy changes, a second
scheduler, subjective productivity scoring, inferring billable hours from a
terminal existing, or implementing a cloud relay to make the phone work remotely.

### Projection shapes and interaction

```rust
struct DeskQuery {
    view: DeskScope,                 // AllWork or Context(context_id)
    lead_ids: Vec<AgentId>, after: Option<RecordId>,
}
struct DeskProjection {
    lanes: Vec<ConversationProjection>, queue: Vec<AssignmentRunProjection>,
    needs_you: Vec<RequestProjection>, activity: ActivityDigest,
    accounting: WorkKindRollup, revision: RecordId,
}
struct PaybackUse {
    system_assignment: AssignmentId, output_run: RunId,
    artifact: ArtifactRef, observed_event: RecordId,
}
```

| Region | Behavior and source |
|---|---|
| Lead lanes | Parallel canonical head conversations; type into another while one runs. Show team/scope, run receipt and budget. Narrow screens use tabs preserving independent drafts. |
| Direct member selection | Open a head's thread or a member's run view without routing through the global head. Viewing never starts another run. |
| Headless queue | Assignment + observed Run state, deadline, last observation, typed blocker, cost/reservation and result link. Dead/stale is distinct from idle/running. |
| Needs you | Typed request summary, options, trade-off, recommendation and time cost; discuss, snooze, answer, approve and deny remain distinct. |
| Activity summary | Default one healthy-fleet line; expand to attributed receipt/event evidence, not a prose-maintained board. |
| Tasks / links | Assignment projection plus references to authoritative external task stores, workspace and attached terminal. No duplicate stint completion state. |
| System \| Output | Required assignment kind, measured/estimated spend, execution time and items; optional payback intent and observed reuse links. |

The visible inbox has a hard cap of **three** items and at most **one hard** item,
following the approved Grok Bot attention pattern. This is a product limit, not
a count of repository artifacts. The full durable request queue is never dropped:
show an overflow count and explicit “all requests” view. Hard safety/irreversible
items rank first, then deadline/blocked descendants, with stable FIFO tie-breaking;
additional hard items wait behind the first without being relabeled soft.
Snoozing cannot extend authority expiry or hide that blocked work remains stopped.

A permission card shows the exact target, actor, changes, expiry and action.
A needs-input card gives context/options; its answer cannot issue a grant.
Discussion appends to the linked conversation and leaves the decision pending.
A desktop and phone racing to answer use expected request version; one terminal
resolution wins, the other sees the existing receipt. No model classifies an
ordinary chat “sounds good” into approval of a different pending action.

Weekly roll-up uses a configured week boundary/timezone and states it. Display
execution duration separately from human attention and provider spend; concurrent
run durations are not elapsed human hours. Unmeasured values say unknown.
`pays_back` is intent, not proof of value. Link system improvements to output runs
that actually used their versioned artifacts via `PaybackUse`; report reuse and
outcomes, never invented dollars saved. Corrections to classification are audited
and recompute projections rather than edit a second ledger.

Phone and desktop query the same inbox, queue, conversation and roll-up API.
The phone owns only view preferences and clearly unsent drafts; it has no
independent receipt, assignment, permission or scheduling state. Reconnect uses
P2's cursor/snapshot contract. P4 may use the local authenticated phone adapter;
public remote availability awaits the separate cloud pairing/HTTPS work.
Do not claim the prototype bearer URL is that security model.

**Design evidence:** Grok Bot's named leads with persistent threads preserve direct
access to Narrative/DU and loop owners; background completion digests preserve
attention; its ranked small inbox and one hard item bound interruptions; explicit
owner/next-action/blocker records make work actionable; exact-text send approval
keeps irreversible rails. Reusable workflows make system investment visible.
Desk keeps these interaction patterns from the approved Munder/teammate package.
It replaces hand-synced markdown boards and shared-account authority with host
records, projections and per-run scope. No external implementation is copied or
assumed to supply the API.

### Migration

Merge the team-plan Desk and proposed command-view shell into this one surface.
Replace hand-maintained queue/inbox/accounting copies with P2 projections.
Existing Assistant panes remain views of canonical heads; they do not write a
parallel conversation store. Phone prototype receipt/transcript authority is
removed in P2 and cannot be reintroduced for mobile UI convenience. Migrate
links/bookmarks by durable IDs, not by guessed display-name matches.

### Acceptance criteria

1. **AT-P4-01 — Parallel lanes:** TOML scene `desk-multiple-leads.toml` sends to
   distinct heads, leaves one waiting for permission and verifies another can
   finish; direct member selection preserves identity and no duplicate worker.
   Verify per-context and All work filtering with authorized seeded teams.
2. **AT-P4-02 — Inbox attention contract:** Rust projection test
   `desk_inbox_caps_ranks_and_preserves_overflow` supplies mixed needs-input and
   permission requests, deadlines and duplicate events; proves the cap, one hard
   item, stable order and lossless overflow. Scene `desk-needs-you.toml` verifies
   labels, keyboard actions and discussion leaving permission pending.
3. **AT-P4-03 — Honest accounting:** Rust test `desk_rollup_uses_run_evidence`
   includes parallel runs, missing usage, retries, corrected kinds and reuse links.
   Verify no double charge, no terminal-time billing, explicit unknowns and
   payback linked to observed output runs.
4. **AT-P4-04 — Same phone state:** live VM step opens desktop and phone, resolves
   one request concurrently, kills a worker and reconnects the phone from an old
   cursor. Both show the same winning receipt, observed failure and remaining
   queue without duplicate approval or private mobile state.
5. **AT-P4-05 — Visual and narrow-screen proof:** scenes for populated lanes,
   long asks, full-trust indicator, empty/overflow/stale queue and System | Output
   render realistic data at desktop and narrow sizes. Inspect screenshots; a
   live VM/phone browser step verifies touch/composer/scroll and keyboard focus.
   Record physical-phone evidence separately from browser emulation.

### Exit gate

In the isolated installed VM, queue output work with two leads and system work
with a third; close the conversations and observe their supervised runs. Kill a
worker to demonstrate observed liveness. Open Needs you on the phone, discuss
without approving, resolve the actual request on desktop, then verify the same
queue and weekly roll-up. Reopen a member terminal/view without spawning work.
Review screenshots of populated desktop/mobile surfaces and the persistent
full-trust indicator; report script/provider modes and device evidence separately.

## P5 — Video editor proof

### Goal, scope, non-goals

Prove the same prepare/review/commit/revert contract on a creative timeline.
The human sees proposed cuts as ghost clips and corrects the canonical project
without translating their edits back into chat.

Scope: minimal timeline with clip order, in/out and placement; plain-text XML EDL,
`video.timeline` and `video.propose_cuts`; ghost preview, per-hunk acceptance,
revision conflicts and reuse of existing video playback infrastructure (E16).

Non-goals: general NLE, effects/color, arbitrary XML dialects, export/render
qualification, full multicam V2/V3 delivery, Linux production decoding, or a
private JSON edit database. Those boundaries do not waive the XML owner's
round-trip and source-frame correctness requirements for supported operations.

### Proposed document and API shapes

The project is the XML spec's Premiere/FCP7-style `xmeml`, used as the plain-text
EDL. The timeline model is derived from it. JSON below is an API projection,
never a competing project file. Preserve stable clip/file IDs, untouched metadata,
audio linkage, media paths, enabled state, order and gaps. Disable external
entities; reject unsupported rates, retiming and constructs or open read-only.

```json
{
  "resource_id": "video_project_opaque",
  "revision": 12,
  "rate": {"numerator": 30, "denominator": 1},
  "clips": [{"id": "clip_A", "source_id": "media_A", "track_id": "track_A",
             "in_frame": 40, "out_frame": 160, "start_frame": 0}]
}
```

| Operation | Contract |
|---|---|
| `video.timeline(resource)` | Read authorized project revision, stable clip IDs, integer-frame positions and explicit rate. |
| `video.propose_cuts(resource, expected_revision, operation_id, cuts)` | Prepare P3 ChangeSet with clip/track/time footprint, dependencies and ghost preview. |
| `changes.commit(id, selected_hunks, expected_revision)` | Conditionally apply accepted cuts to XML through the same commands as human UI. |
| `changes.revert(id, expected_revision)` | Independent inverse or compensating preview; never replace newer human XML with a snapshot. |

Resolve selection to stable clip/track IDs and an explicit source/project time
basis before granting. Ripple edits must include every moved clip in the write
footprint; a selected-track grant cannot silently affect another track. Prepare
is immutable; accept/reject selects a dependency-closed set with a new commit
digest. Moving a clip while a proposal waits produces `edit_conflict` and a fresh
preview. No approval can silently retime an old operation onto the new project.

Render accepted clips normally and proposed clips as unmistakable ghost overlays;
show source frames, actor/run, changed timing and accept/reject controls per hunk.
Keyboard equivalents cover every mutation. Playback reuses the existing player
and `VideoDecoder` service/shared clock rather than growing another decoder stack.
Keep sources untouched; save named XML revisions atomically. External valid edits
reload a clean document; dirty/revision conflicts preserve both choices for
reload/Save As, following the XML owner. Invalid XML leaves the last valid preview.

Production playback currently depends on the macOS decoder. Linux can test
parsing, authorization, transactions and mock frame behavior, but a mock pass is
not Linux media support. Inject `MockVideoDecoder` for deterministic tests; run
real AVFoundation playback on a qualified macOS VM/host for final evidence.
If no macOS VM is available, record that live requirement as unmet; never relabel
a Linux mock as a completed playback gate.

### Migration

Reuse P3 ChangeSet and P2 audit/identity records. The XML spec remains authority
for the document and interoperability; this phase narrows its first agent proof,
not its final V2/V3 acceptance. Do not add a second cut-list schema or write
compatibility shims for speculative JSON projects. Existing player callers
retain playback behavior while the timeline uses its service through an explicit
adapter. Unsupported edits fail with named limitations, not lossy conversion.

### Acceptance criteria

1. **AT-P5-01 — XML/timing invariants:** Rust table test
   `video_changeset_xml_roundtrip` verifies no-op save, repeated trim/undo,
   stable IDs, gaps, links, supported integer-frame timing and rejection of
   external entities/unsupported rates/retiming. Compare parsed semantics and
   preserved untouched metadata, not merely successful XML parsing.
2. **AT-P5-02 — Footprint and conflict:** HostHarness test
   `video_proposal_cannot_overwrite_human_clip_move` sends a real tool proposal,
   changes placement through the human command path and commits. Assert
   `edit_conflict`, unchanged human placement, and no unauthorized ripple onto
   another track. Duplicate accepted operations commit once with attribution.
3. **AT-P5-03 — Ghost preview:** TOML scene `video-agent-ghost-clips.toml` uses
   synthetic media/mock decoder, shows realistic clips and proposals, accepts
   one independent hunk, rejects another and reverts an independent agent edit.
   Semantic assertions plus reviewed screenshots prove the ghost/real distinction.
4. **AT-P5-04 — Installed media proof:** on a qualified macOS VM/host, open the
   fixture XML, request cuts from an agent, scrub ghost preview, accept/reject
   hunks, make a human correction, and observe conflict on stale commit.
   Save/reopen and verify intended source frames and timing through real playback;
   record seek/A/V observations and decoder identity. A Linux mock run is separate.
5. **AT-P5-05 — External edit safety:** Rust tests plus a live VM file-reload step
   inject invalid XML, dirty-buffer edits and interrupted save. Last valid
   project/media remain intact, conflict offers recovery, and no silent overwrite
   or source-media mutation occurs.

### Exit gate

AT-P5-04 is the creative proof: attributed proposal, visible ghost clips,
per-hunk decisions, human correction and conflict all occur on the same XML
project. Capture resource/ChangeSet/run/grant IDs, before/after revisions and
frame observations via sanctioned tools. Review the timeline screenshots and
record real-decoder evidence separately from mock coverage. This gate does not
claim final export fidelity or the XML PRM's full multicam release.

## Drift disposition

This maps every item in alignment's “Where they drift.” It assigns ownership,
not progress state. Deferred work has a reason and is not part of a passed gate.

| Drift | Resolution / phase | Boundary or deferral |
|---|---|---|
| 1 — Split authority across hooks, AgentHost, subscriptions and launch; MCP/CLI bypass | P1 replaces permission decisions with the mandatory monitor in dispatch, including launch/subscription host operations and shared grant store. | Runtime capability checks and app legality remain defense in depth, not competing authorizers. |
| 2 — Rich grants ignore resource/duration | P1 matches complete binding and enforces once/session/expiry; replacement and session cache use the same key. | Old broad grants are invalidated unless a signed-off explicit reissue flow replaces that default. |
| 3 — LAN phone vs paired HTTPS relay | P2 fixes durable receipts, addressed intake and real cancellation; P4 displays canonical queue/inbox. | HTTPS, single-use pairing, device revocation and outbound relay remain cloud phase P4 work. Deferred because this spec proves local host behavior and authorizes no deployment. Prototype bearer/LAN is explicitly not equivalent. |
| 4 — Text to first matching pane | P2 versioned envelope addresses opaque agent/conversation with request ID, expiry and target revision. | P1 uses one isolated Assistant only for its existing phone demo; this is not the final routing model. |
| 5 — Channel-specific agents vs neutral skills | P2 moves definitions/team to `.plexi/agents` and `.plexi/team.toml`, adds `/agent new --here`, explicit import and ownership lease. | Credentials/operational channel state are not indiscriminately moved. |
| 6 — Lossy events and volatile timeline | P1 durable correlated authority audit; P2 lossless agent drain, IDs, versioning, recovery; P3 persistent checkpoint references and rollback. | Ordinary UI telemetry may remain best effort. App state remains app-owned, not duplicated in the drain. |
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
| HostHarness | Production command/tool ingress, real app runtime, scripted Assistant loop, correlated replies/events and hidden/inactive/no-view service paths. |
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

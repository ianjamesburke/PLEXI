Status: active
Stint: none yet

# Agents API and one permission gate (P1–P5)

## Purpose and product story

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
| [Run orchestration](../agent-run-orchestration.md) | Retains observed liveness, leases, bounded supervision, and reconcile-on-restart. P2 supersedes names-as-identity and independent authoritative per-run journals for agent work. |
| [Host Assistant](../assistant-host-app.md) | Retains tools, content blocks, permission UX, settings and backend separation. P1 replaces optional permission hooks; P2 replaces pane-owned intake and channel-specific agent definitions. |
| [XML video editor](../xml-video-editor.md) | Owns XML dialect, timing, round-trip and playback/export fidelity. P5 adds ChangeSets; it does not replace XML with a private EDL database. |
| [Testing](../../src/testing/TESTING.md) | Owns implementation build gates, Rust/scene split, isolation, editor gate, and mandatory visual review. |

## Glossary

| Noun | Meaning and boundary |
|---|---|
| Principal | Authenticated human, device, agent, or application identity. Origin is stamped by the host, never asserted in message text. |
| Assignment | Goal, owner, resource/action scope, deadline and budget; required `kind: system\|output`, optional `pays_back`. |
| Delegation | Host record allocating a subset of an assignment to a child; intersection can preserve or reduce rights, never add them. |
| Run | One supervised execution attempt of an assignment, with observed lifecycle, pinned configuration, reservation and typed result. |
| run token | Revocable host-issued credential bound to principal, agent, run, assignment, delegation and host generation. Possession does not bypass live policy checks. |
| Message + receipt | Attributed speech act and durable delivery/execution state. Information is not delegated authority. |
| Request | Typed unresolved human decision: `needs_input` or `permission`. Discussion, snooze, reading, and approval are different operations. |
| ChangeSet | Prepared delta against an app-owned resource, with revisions, footprint, preview, selected hunks and attributed commit/revert receipts. |
| Event / drain | Host-stamped, versioned, addressable record and the lossless append/read service for agent records. Bus delivery and UI projections derive from it. |

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

|---|---|---|

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

### Human-owned pane authority and context scoping

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
| Socket identity caveat (inventory) | P1 host-authenticates peer/session, rejects claimed pane impersonation and removes no-pane `user`. | Local OS-user access to the socket is not proof of human interaction or an agent's assignment. |

P1 documentation work must replace mesh appendix H6's raw-path dispatcher claim
with context-owned `evaluate_reach` (E1), and remove phone README's opening
“no auth/no host connection” claim in favor of its real prototype limits (E17).
Qualify “no Railway” wherever the architectural provenance is reused: website
configuration exists (E19); a phone relay is separate work. These corrections are
deferred to P1 to keep this commit's explicitly requested two-file scope.
Mesh's older “no Assistant CLI” claim also needs replacement with E12's limited
pane-addressed intake. Do not copy the appendix as a current-code inventory.

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

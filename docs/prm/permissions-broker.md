# Thin Plexi core and core plugins on one capability API

Status: active

Stint: 1182 (design); 1183 (back burner), 1140 (attention), 1151 (commands), 1207 (public UI), 1201 (undo). Additional execution scopes are drafts in the appendix.

## Decision

Extend the existing app model to contribute host surfaces. Call the package a **plugin** and its pane surface an **app**. Keep one manifest, identity, lifecycle, capability vocabulary, broker and distribution path. A plugin may contribute a pane, command, panel, lifecycle subscription or theme without acquiring another identity or more authority.

The core owns terminal execution, pane identity and layout, input routing, rendering primitives, persistence transactions, resource scheduling, capability enforcement, trusted consent and the command registry. Bundled plugins own the replaceable workflows built from those services. They ship inside the Plexi distribution, with readable example source, and use exactly the public API available to an independently installed package. Bundling affects availability and provenance; it grants no permissions.

For V1, build the public contract around Python in the existing Wasmtime sandbox, move guest execution into supervised helpers, and extract the day-to-day features listed below. Keep native WASM experiments on the same semantic contract without expanding their V1 product scope. Marketplace distribution and revenue remain Phase 4 work.

This document restores the path referenced by `src/broker/mod.rs` and replaces the deleted broker proposal from `7c62636c`. Its broker section supersedes the old migration/default-allow policy. It specifies the destination; it does not claim the API or isolation changes already exist. The task graph owns progress. Before closing the design task, link approved execution tasks here; delete this PRM only with its last execution task, after moving durable contracts beside their owners and repairing incoming links.

## Evidence and decisions from the design inputs

The inventory was checked against source commit `750eaa9677f8dca233175af149234431d8f4ad5f`. This is source evidence, not installed or measured runtime evidence. The relevant product direction is [NORTH_STAR.md](../../NORTH_STAR.md), particularly the V1 progression and commandments on commands, authority and ownership. The release owner's `TRUTH.md` places the plugin platform after the 0.4.4 patch snapshot; this design does not pull automatic ask reveal into that patch.

The supplied audits are local design evidence in the main checkout's git-ignored task store. Their recommendations are incorporated here; they are not prerequisites a plugin author must locate:

| Input | Decision carried into this contract |
|---|---|
| `.stint/runs/design-20261109/asks-ux-audit.md`, sections on operation state, typed cards and common projection; stint 1140 | One source-owned attention projection, host-owned consent cards, correlated operation status, atomic orphan cancellation and verified history. The inbox plugin presents requests; it cannot manufacture approvals. |
| `.stint/runs/design-20261109/context-roots-audit.md`, proposed per-app binding | Separate owning context, package directory, working folder and state address. Add explicit Open in folder and inspectable bindings. Shell `cd` proposes a folder; it never retargets a running app. |
| `.stint/runs/design-20261109/ui-layout-safety.md`, contract and migration; stint 1207 | Publish safe rows, action toasts, headers and surface color roles through both public transports. Reuse the host kit after its API settles; do not copy unmerged symbols from the 1202/1206/1203/1204/1205 lane. |
| Stint 1201 | Plugin undo contributions join host history through validated inverse operations. Text undo retains first refusal. Undo of a running pane's move is possible; revival of an exited process is not. |
| Stint 1183; stint 0896 and `.stint/plans/HIDE-PARK-CONSULT.md` | Adopt layout/lifetime separation and attention on failure. The newer back burner ruling replaces the consult's hide/unhide naming, aliases and restore-to-slot requirement. |
| Stints 1151 and 1175 | Use the shared command registry and existing typed CLI operation admission. Do not build a second dispatcher or an unrestricted shell escape. |
| Stints 0788 and 1179 | Keep secrets out of agent environments, preserve trusted host consent, and preserve channel-owned secret namespaces. Plain user documents remain channel-neutral. |

Load-bearing choices:

| Choice | Alternatives | Recommendation and reason |
|---|---|---|
| Extension identity | Separate native extension system; extend apps with contributions; load arbitrary native libraries | Extend apps. A second system would duplicate grants, packaging and UI. Native libraries cannot enforce the public boundary or isolate failures. |
| Authorization | Keep per-runtime stores; put every setting and notice into one store; one broker with typed resource adapters | One broker. Resource owners validate operations; only the broker decides authority. Presentation and requested policy keep separate data models. |
| Working location | Context-only execution; explicit per-instance folder; general mounts | Explicit folder, defaulting to the owning context root. It solves the reported wrong-folder board without adding a mount namespace. |
| Guest scheduling | Synchronous guest calls; cancellable in-process workers; supervised Wasmtime helpers | Helpers for V1 Python. They add startup/IPC cost but give a reliable kill boundary for stuck guests and decoders. Worker threads alone cannot safely terminate a wedged host import or blocking join. |
| UI composition | Raw guest pixels; host semantic components; arbitrary native view hooks | Host components plus a bounded canvas escape for content. Host geometry must own paint, input and accessibility together. |
| Consent presentation | Plugin-owned allow buttons; entirely native inbox; replaceable inbox with host consent surface | Replaceable inbox with host consent. It permits customization without giving a plugin the power to sign its own decision. |

## Inventory: present surfaces and duplication

Paths and symbols identify the inspected code, rather than promising those symbols as public API.

| Surface and source | Present responsibility | Duplication or gap to remove |
|---|---|---|
| [Broker](../../src/broker/mod.rs): `GrantStore`, `GrantRecord`, `PermissionRequest`; [gate](../../src/broker/gate.rs): `PermissionMonitor`, `AdmitRequest` | Generalized actor/action/target/scope records, exact admission, sealed grants and audit, pending decisions | The module header says Phase A, but CLI operations and other adapters have grown beyond that description. Legacy migration remains in `GrantStore`. Several callers still perform independent capability decisions. |
| [Legacy permissions](../../src/app/permissions.rs): `PermissionStore`, `PermissionState`, `Capability` | Green/Yellow/Red app/workspace decisions and capability vocabulary | Legacy readers/writers coexist with broker grants. `WasmPane::decide_next_capability_prompt_inner` updates session state, the legacy store and the monitor. This is dual authority, not merely duplicated UI. |
| [Manifest/registry](../../src/app/registry.rs): `AppManifest`, `AppCapabilities`, `SecretDecl`, `RegistrySource`; [package](../../src/app/package.rs): `trust_label` | Package declarations, launch policy, registry precedence, review/provenance labels | `LivePythonPane::has_capability` treats manifest membership as a grant. Package trust labels must not become bypasses. Code discovery and execution folder selection are also entangled. |
| [Agents](../../src/agent/mod.rs): `AgentDefinition`, `AgentHost`; [runs](../../src/agent/runs.rs): `AgentAuthorityRecord`, `ToolGrant`, `RunAuth` | Definition requests, run identity, delegated authority and broker integration | Agent posture, run grants and `AgentHost`'s store view are additional admission inputs. Consolidation must preserve narrowing delegation and remove any independent widening path. An agent card remains discovery only. |
| [Attention](../../src/app/attention_queue.rs): `QueueSource`, `sync_attention_queue`; broker `NeedsYouKind`, `NeedsYouRecord`; [approval keyboard](../../src/app/approval_keyboard.rs) | Projects broker asks, notifications, observed agent state and child results into UI | Projection is not universal source ownership. Event consent and native WASM prompts have separate surfaces. A local dismiss cannot stand for provider response or grant revocation. |
| [Secrets](../../src/workspace/secrets/resolver.rs), [pane reads](../../src/workspace/secrets/pane_read.rs), [secret get](../../src/app/secret_get.rs), [Secrets UI](../../src/app/secrets_app.rs) | Resolve canonical names, access backing stores, gate pane reads, display settings | Manifest secret declarations, terminal injection, SDK access and UI must share one reference/admission route. Storage ownership remains separate from general user-data tiers; 1179 addresses cross-signer writes. |
| [Runtime reach](../../src/host/scope.rs): `ScopeOrigin`, `evaluate_reach`; [file addresses](../../src/host/state_scope.rs): `StateScope`; registry `RegistrySource` | Context-ID reachability, channel-neutral document locations, channel-scoped config shadowing | These are intentionally different models. Reusing “workspace” for all three causes wrong targets. Live context root and captured execution workspace currently diverge in Python; native WASM state uses another address scheme. |
| [PGAP](../../src/protocol/commands.rs), [Python SDK](../../sdk/python/plexi_sdk), [Python bridge](../../src/host/wasm_python.rs) | Typed requests/events, declarative UI and host effects over the Python bridge | Large bridge match arms enforce some manifest-only gates and execute some synchronous file effects. PGAP and WIT do not yet have identical effects or grants. |
| [WIT](../../wit/plexi.wit), [WASM app](../../src/host/wasm_app.rs), [WASM pane](../../src/host/wasm_pane.rs) | Component lifecycle, linked imports, effect loop, host UI renderer | Link-time access and runtime grants are different checks. Direct component lifecycle calls lack the Python path's worker separation. Missing plugin variants must be versioned, not silently tunneled through strings. |
| [Approved CLI operations](../../src/host/cli_ops/service.rs): `CliOpService`, `PaneIdentity`, `binding_digest`; [executable admission](../../src/host/cli_ops/exec.rs) | Typed argv without a shell; exact package/workspace/definition/executable binding; bounded execution | Reuse this service. It has final results but needs public intermediate status and joined cancellation with the broker. A declared read/write scope is disclosure, not OS confinement. |
| [Commands](../../src/host/keys.rs): `Action`, `build_key_bindings`, `build_binding_table`; [palette](../../src/overlays/command_palette.rs): `PALETTE_COMMANDS` | Keyboard dispatch and palette discovery | Separate metadata and dispatch entry points are the duplication 1151 removes. Plugin commands join that registry. |
| [Builtins](../../src/pane_ops/create.rs): `builtin_factory`; [bundled pack](../../packs/core.toml); native overlays | Compiled Rust apps, extracted Python apps, and workflow UI inside `PlexiApp` | Native features can access host internals unavailable to packages. Extract workflows and publish missing primitives instead of preserving private shortcuts. |

The legacy-store removal census is the production references in `src/app/lifecycle.rs`, `src/broker/mod.rs`, `src/cli/app.rs`, `src/cli/open.rs`, `src/host/wasm_pane.rs`, `src/host/wasm_python.rs`, `src/overlays/confirmations.rs`, `src/pane_ops/create.rs` and `src/workspace/mod.rs`, plus its definition and fixtures in `src/scenes.rs` and `src/ui_tests.rs`. Re-run the symbol search during removal; a source reference is not automatically an active admission path.

### What is isolated today

The [runtime reference](../wasm-runtime.md) describes the intended shared app contract, but some of its “same component boundary” and “same grant flow” wording is stronger than the source. `WasmPythonRuntime::launch` creates a raw WASI Preview 1 CPython module with read-only `/app`, `/sdk` and standard-library mounts, a `Store<WasiP1Ctx>` and PGAP stdin/stdout. Native `WasmApp` uses the component lifecycle in WIT. Both use Wasmtime memory isolation; they do not currently use the same wire ABI or admission flow.

| Runtime | Source evidence | Isolation limit |
|---|---|---|
| Python via `wasm_python` | `WasmPythonRuntime::launch` runs `_start` on its own thread; `PythonOutputDecoder::spawn` decodes output separately; `drain_runtime` uses nonblocking receives | Guest compute is off the UI thread. `Drop` joins the guest thread without a deadline; `handle_file_read` and `handle_file_write` perform synchronous filesystem calls; queues and tree/render work still need aggregate bounds. This is not a guarantee that a slow plugin cannot stall a terminal. |
| Native WASM | `WasmApp::init/update/view` call component exports directly; `WasmPane::drain` calls `app.update`; `LiveWasmPane::background_tick` calls tick and view inline | Linear memory and linked-import isolation exist. Lifecycle compute can block host servicing. The inspected `wasm_engine::create_engine` configures neither fuel nor epoch deadlines. |
| Compiled builtins | `builtin_factory` returns native `App` implementations | Share the host's process and internal interfaces. Public-API and scheduling isolation are absent by construction. |
| Brokered CLI and configured MCP | CLI service launches approved programs; `McpConnection` owns external processes | Process execution is separate, but native programs run with account authority. Plexi checks admission and lifecycle; neither cwd nor an app grant supplies an OS sandbox. |

The V1 performance claim therefore requires new work. A prior balls FPS sample in the runtime reference cannot prove shutdown, malicious guest or terminal-latency isolation.

## Public contract

All API names below are proposed. The contract is semantic: PGAP's typed schema and Python SDK and the versioned WIT definitions must describe the same operations, errors, identities and lifetimes. Wire encodings may differ. Do not introduce a plugin-specific JSON escape hatch or a parallel `plugin install` ecosystem; extend app package commands and discovery with contribution metadata.

Use a typed request envelope with a caller-local `request_id`, an optional idempotency key, operation variant, explicit target references, expected binding/resource revisions and variant-specific arguments. The host adds authenticated principal, instance generation and delegation provenance; these fields cannot be supplied as authority by the guest. Responses contain the request ID, host-issued operation reference and either typed data or a stable error code with a redacted explanation. A retry with the same key and different arguments is rejected. IDs are names, not transferable bearer authority; each use checks the caller and current generation.

| Proposed operation family | Capability boundary | Result/resource |
|---|---|---|
| Instance/binding inspection and retarget | Own nonsecret identity is core policy; observing others or retargeting needs scoped admission | Revisioned binding or a typed target error |
| Files/documents/state | Separate read, write, watch/search and trash/restore rights to named collections or handles | Bounded bytes, document versions, cursors or transaction receipts |
| Panes/rooms/lifecycle | Read, subscribe, move, recall and close are distinct; cross-context reach is additional | Snapshot/cursor, cleanup lease or committed layout revision |
| Commands/panels/presentation | Registration covers declared contributions; invocation checks the operation's authority | Invocation result or revocable surface/presentation lease |
| CLI/network/secrets/media | Existing typed resource adapters, narrowed to declared program, destination or resource | Correlated operation and bounded typed output/opaque handles |
| Attention/permissions/history | Publish own questions/notices, read admitted items, review or cancel; authority mutation remains protected | Source-owned item, host review surface or verified audit page |
| Undo | Contribute own entries; executing inverse/redo rechecks underlying rights | History entry and acknowledged outcome |

Extend the existing PGAP types and WIT interfaces together, publish the Python SDK from those definitions and check both against shared semantic fixtures. WIT remains authoritative for component wire types; PGAP remains authoritative for its encoding. A missing mapping is an explicitly unsupported feature, never success with a dropped effect.

### Package, manifest and lifecycle

Extend `AppManifest` with an explicit API major, runtime requirements, contribution declarations and requested capability/resource patterns. Keep package ID, version and verified content digest separate. The host derives the digest and provenance; a package cannot claim to be bundled. Contributions use package-namespaced stable IDs.

| Manifest field | Meaning |
|---|---|
| API and runtime requirements | Required contract major and supported features. Reject unsupported mandatory features before activation with a typed error. Optional contributions may be unavailable with a reason. |
| Entry | Python entry for V1 code plugins; a data package has no entry or executable. Existing experimental WASM targets declare their world and feature requirements. |
| Contributions | Panes, command handlers, host panels, settings forms, file handlers, lifecycle subscriptions, undo handlers, or theme data. Each declares scope, activation trigger and stable ID. |
| Capabilities | Requested actions and resource patterns, with human-readable reasons. These are ceilings on requests, never grant records. |
| State | Declared global/context scopes and portable format, through the existing user-data resolver. Document content uses document references, not an invented per-channel plugin directory. |
| Secrets and CLI operations | Secret names/purposes and typed operation declarations, admitted through their resource adapters. No plaintext credentials or arbitrary executable paths. |

Use a host-managed instance lifecycle: registered, activating, ready, stopping, stopped or failed. Activation happens on a command, surface open or admitted subscription, rather than eagerly starting every installed package. A theme never starts a runtime. A context service instance can own several transient surfaces; closing its panel need not end its service. The host enforces instance and resource quotas. Package disable/uninstall stops instances, cancels operations, unregisters contributions and retains user documents unless the user separately deletes them.

The host sends an initialization snapshot containing instance identity, owning context, folder/state bindings, admitted handles, config revision and available API features. Updates deliver typed events; views return declarative trees; effects request operations. Shutdown is advisory to the guest and bounded by the supervisor. A new instance generation invalidates old actions and handles. Upgrade quiesces the old generation, saves portable state, swaps atomically, and re-admits changed authority. A failed upgrade leaves the prior package available; it never silently broadens its grants.

### Identity, scopes and folders

Every request carries a host-stamped caller principal and generation. A plugin-supplied context, pane or folder is a target to resolve and authorize, never caller identity. Context ownership and cross-context reach follow [the host scope contract](../../src/host/AGENTS.md). Package provenance is a separate field, never a scope or trust shortcut.

Extend the host origin model to identify context-service instances without inventing a hidden pane to hold their authority. Pane identity is optional for such instances; authenticated instance identity is mandatory. When an agent or another plugin invokes a contributed command, carry its delegation chain and admit the intersection of caller delegation, plugin ceiling and resource grants. A plugin's remembered grant must not become an ambient privilege the invoking agent can borrow. Host-created scheduled work uses its own explicitly approved activation scope.

The host issues an `InstanceBinding` with `instance_ref`, `context_ref`, `folder_ref`, `state_scope` and `binding_revision`. `folder_ref` is a canonical resource reference owned by the binding; it does not grant filesystem or CLI access. State addresses continue to use the channel-neutral resolver. Runtime reach uses context IDs even when two contexts have the same folder. Config discovery continues to shadow by registry source.

Global state is still package-owned state, not permission to read every package's files. Shared collections such as the existing notes tiers require an explicit collection grant. Keep structured app-to-app events on the event bus or directed JSON pipes and bulk media on typed pipes; pane lifecycle subscriptions do not create a fourth data transport.

`app.open` takes explicit owning context and optional working folder. The default is that context's root, independent of focus and package installation location. CLI `app open <id> --folder <path>` and the palette's Open in folder invoke this operation. `--folder .` deliberately captures CLI cwd. Instance reuse matches package, context, folder and surface; an A-bound instance cannot satisfy a request for B.

`binding.inspect` is available to the instance and authorized observers. Host app chrome always shows the working folder; structured pane state exposes it and the state scope. `binding.retarget` shows old/new targets, stops new admissions under the old revision, cancels pending approvals and creates new requests only after revalidation. Running operations retain their original target and visible status. Old results cannot update a newly bound document. A root change emits a revisioned scope event and cancels stale state writes; it does not move a running shell or silently copy data. Restoring a missing folder reports an error, with an explicit correction flow, rather than falling back to home.

### The one broker

`PermissionMonitor` becomes the sole admission authority for public operations. `GrantStore` persists its decisions. Resource adapters normalize requests and perform checks specific to files, secrets, panes, tools and external programs. Agent definitions, manifests and settings constrain requests; they cannot mint allow records. Existing delegated run authority becomes a broker-verified narrowing reference to its parent, with explicit revocation and expiration.

Admission proceeds as follows:

1. Authenticate caller/channel/generation and resolve explicit targets. Reject forged, stale or unreachable references before prompting.
2. Validate typed arguments and manifest ceilings. Capture canonical resource identity, argument fingerprint, binding revision, package digest and operation definition. Resources that can change at execution time need version preconditions.
3. Evaluate applicable managed constraints, explicit denies and ask policies before allows. A workspace/package file cannot supply an allow. Implicit operations such as reading an instance's own nonsecret identity are an enumerated core policy, tested for every runtime; unspecified protected operations default to ask or deny.
4. Match exact grants, with the disclosed duration and argument/resource boundary. Once grants authorize one admitted execution. Session grants expire with the host session. Remembered grants carry a concrete expiration; “Always” in existing enums must not imply infinity in UI. Preserve the current 1175 operation binding instead of rebinding remembered CLI grants to ephemeral pane IDs.
5. If consent is required, persist a pending request and return `waiting_for_approval`. Only trusted host input through the protected consent renderer may commit an allow. Neither guest actions, socket calls, terminal injection nor a missing pane identity count as a human approval.
6. Persist and audit before releasing authority; failure leaves the operation unexecuted. At execution, recheck identity, revocation, targets and preconditions. Consume a once grant atomically with admission; retries use an idempotency key and cannot reuse it for changed work.
7. Audit execution outcome separately from consent. Store correlation IDs, operation/resource identity, duration and result category, with secrets and sensitive payloads redacted.

Live resource handles are revocable and scoped to the admitted operation or instance. Linked WASM imports are only a transport boundary: a linked import must still validate the current handle. A revocation prevents new work and attempts cancellation of running work; already completed external effects cannot be undone by revoking consent. Package replacement, folder retarget and host restart invalidate pending approvals unless the host can revalidate their exact continuation. No blanket “core plugin trusted” branch is allowed.

Retire `PermissionStore`, dual writes, Python manifest-as-grant checks and runtime-specific approval queues. Preserve independently verified exact broker grants; invalidate legacy records that cannot prove the new binding, retain an inert export for inspection, and ask again. Do not ship a fallback reader or compatibility grant path. One-time user-data recovery and 1179's audited secret migration are distinct from permission compatibility.

### Operation state, secrets and attention return

An effect submission receives an `operation_ref`. `operation.status` and ordered status events report `queued`, `waiting_for_approval`, `running` and a terminal result (`succeeded`, `failed`, `denied`, `cancelled`, `expired`, or `outcome_unknown`). Include request correlation, sequence, binding revision and an optional attention reference. Status queries reconcile missed events; delivery is replayable, execution is deduplicated. Approval alone never produces success.

Cancel, timeout, pane death, instance death and plugin disable share one owner transition that retires pending authority and the continuation before acknowledging cancellation. A late click is rejected. If a native program has already made changes, report cancellation with possible partial effects instead of promising rollback. The host supplies a waiting strip even if the guest ignores status events; it disables only the affected action and leaves other controls usable. External drains run in `App::logic`, with inactive-context servicing through `background_tick` or its successor.

`secret.resolve(name, purpose)` yields an opaque `SecretRef` bound to principal, resource, channel and intended use. `secret.use` injects it only into an admitted host-owned request, such as a credentialed HTTP operation. The host performs substitution without delivering bytes to plugin UI, logs, clipboard or agent environment. V1 does not add a general credential proxy or automatic injection into arbitrary native CLI programs. Any exceptional raw read keeps its distinct broker action and trusted disclosure; Secrets UI edits use a host-owned secure input field whose value is never returned to the plugin. Rotation/revocation invalidates affected handles. Follow 1179's channel ownership; do not search other channels when a secret is absent.

The attention API exposes a versioned projection with `attention_ref`, source owner, host-bound origin, kind, state, permitted actions, disclosure and delivery receipt. Plugins can publish scoped questions, notices and failures, inspect admitted projections, request review and cancel their own work. Only the broker publishes Plexi grant actions. A supported provider adapter owns provider prompts and acknowledges actual delivery; closing an inbox item is not a provider response.

`attention.review` opens a host-owned consent surface by reference. The host renders identity, concrete target/arguments, workspace, exact remember scope/duration and native-program authority from validated records. Plugin explanations are attributed text. Detailed package/program hashes and resource declarations remain inspectable. Plugins cannot supply consent button callbacks or desktop decision tokens.

The bundled Needs you plugin implements 1140's scoped reveal: eligible requests in the foreground context reveal without stealing pane typing; other contexts receive quiet markers. Minimize lasts for the visit, and entering a context can reveal unresolved requests according to the setting. Global interruption needs a separate grant. A minimal core recovery command can open pending trusted consent and disable a failed inbox plugin. It contains no alternative workflow policy. Verified, paginated `permission.history` exposes consent/use/outcome after once grants disappear, without returning active authority or recreating grants.

### Commands, panels, safe UI and pane hooks

Plugin commands register with 1151's registry: stable ID, label, aliases, argument schema, target requirements, availability reason, capability requirements and handler. Keybindings, palette, CLI and agent calls invoke that record. Capture invoking context, pane selection and window before opening a picker; revalidate on dispatch without substituting current focus. Unavailable commands remain discoverable. A command listing conveys no authority, and protected approval is an explicit non-dispatchable exception. The clean-break plugin cutover removes hide/park aliases even though 1151 otherwise preserves unrelated binding compatibility.

Panel contributions use host-owned modal, sheet, sidebar or pane slots. The host owns bounds, focus-stack participation, Escape behavior, safe areas and restoration of prior focus. A plugin receives typed actions, paste/drop resource references and values within its surface. It cannot draw over trusted chrome or intercept other panes' input. A stalled plugin cannot prevent the host from dismissing its surface.

1207 publishes the following semantic components, backed by the host kit rather than a separate plugin implementation:

| Component | Public inputs | Host guarantee |
|---|---|---|
| Safe row | Stable slot IDs, leading/trailing actions, one flexible text slot, optional items and collapse policy | Measure required controls first; allocate text last. Paint, hit testing and accessibility use the same layout result. |
| Action toast | Message, operation/undo reference, named action and severity | Reserve the action; elide or wrap the message; stack on narrow widths. Never present a blank action button. |
| Pane header | Title, subtitle, status, approved accessory/action IDs | Preserve required navigation and actions, use declared icon/overflow alternatives, respect host exclusions. Plugins cannot replace host identity or approval marks. |
| Surface/color roles | Semantic role and state, optional package theme | Resolve against the final composited fill: small text and badges at least 4.5:1, meaningful non-text control marks at least 3:1. |
| Text/document surface | Document reference, edit policy, local undo ownership | Host text input, selection, IME, paste, accessibility and bounded incremental updates; plugin controls document workflow. |

When mandatory content cannot fit, the host reflows, presents accessible overflow or reports `insufficient_space` to a parent that enforces a minimum. A required action cannot silently vanish. Geometry diagnostics expose stable surface/slot IDs, paint/hit bounds, fit decisions and effective colors. Wire validation limits tree depth, node count, text bytes, canvas commands, images and updates before host allocation. Unknown nodes or invalid constraints yield typed errors. Raw canvas is confined to content bounds and cannot claim the semantic layout/contrast guarantee. Reduced motion, keyboard access and accessible labels apply across host and plugin callers.

Pane subscriptions deliver revisioned `created`, `moved`, `room_changed`, `exit`, `closing`, `closed` and `scope_changed` events to admitted observers. Snapshot plus cursor supports reconnection; overflow produces a resync requirement. Events describe committed state and cannot veto core progress. Commands include explicit pane/context references and expected revisions. A plugin can request bounded pre-close work, but cannot hold a terminal close or core shutdown indefinitely. Host-owned rooms retain live pane resources outside the layout; their behavior belongs to plugins.

### Undo contribution point

1201 owns the history engine. A plugin contributes an `UndoEntry` only after a successful operation, containing a plain description, context, originating operation, resource versions, inverse/redo handler IDs, bounded opaque payload and expiry. The host issues the entry ID; the plugin cannot edit another contributor's history.

Undo/redo validates both resource preconditions and current capability grants, invokes the contributor asynchronously and moves the history cursor only after confirmed success. A stale file, missing plugin, closed process or revoked grant leaves the entry visibly unavailable. No success toast appears before acknowledgement; success says “Undid: <action>”. Crash during an external inverse can yield `outcome_unknown`; do not blindly replay it. New work after undo clears the redo branch. Recommend bounded, context-owned session history with a global fallback for genuinely global operations; 1201 settles its cap and persistence. Cmd+Z goes to a focused editing surface first. Document-local edit history stays with that surface; plugin contributions cover workflow operations such as trash, rename or pane moves.

## Performance isolation required for V1

A supervised helper hosts each active code-plugin instance's Wasmtime store and decoder. It receives a narrowly authenticated IPC channel, read-only package/runtime material and explicit handles, with secrets and host socket authority removed from its environment. It cannot directly mutate host state. The helper remains an internal runtime mechanism behind the existing lifecycle; it is not a second plugin contract or a new agent sandbox.

The host never waits for guest work, helper shutdown, compilation, file/network I/O or decoding on the UI/terminal servicing path. Requests enter bounded queues; results return by nonblocking drains with per-instance and total work budgets. Coalesce view updates to the newest complete revision, preserve ordered control/results, and reject overload explicitly. Apply CPU/memory/IPC quotas and admission limits so many well-formed plugins cannot exhaust the host by multiplying their individual budgets. Use fair scheduling with reserved terminal service capacity. Cancellation and helper reaping run outside the UI path; a watchdog kills a stuck helper and leaves a host-owned failure card and recovery commands.

Host effects need their own isolation. Blocking filesystem, executable hashing, secret store calls and media work run on bounded service workers, with cancellable operation records. Bound tree validation, layout and painting as well as guest execution. A small JSON tree containing enormous text or media must not defeat the host budget. Repaint pacing follows existing visibility and reduced-motion rules. Warm pools and content-addressed compile caches may reduce startup cost after measurement; they cannot share plugin memory or authority.

Native WASM remains experimental until it meets this scheduling boundary, including link-time interfaces and audio/GPU work. V1 Python extraction must not wait for new non-Python runtimes. The contract and conformance fixtures reserve the same lifecycle/effects across both; experimental native calls must never be marketed as equivalent performance isolation while they remain inline.

Qualification must measure terminal input-to-paint latency and PTY drain progress beside a guest that loops, blocks a host effect, floods output, sends maximum-sized UI and ignores shutdown, including simultaneous instances, inactive contexts and an occluded host. The proposed acceptance budget is no guest-attributable host service slice above 2 ms and terminal p99 input-to-paint regression no greater than one display frame versus the same fixture without plugins. Record machine, display cadence, workload and distributions; calibrate a stricter supported envelope if needed, rather than accepting a failed budget. Close/disable must finish within the supervisor's declared deadline. Negative controls restoring inline lifecycle calls and synchronous joins must fail. This bounds plugin-caused stalls under supported load; it does not promise real-time behavior under arbitrary OS resource exhaustion.

## Bundled core plugins and the core boundary

The maintained membership source remains [packs/core.toml](../../packs/core.toml). Extend its packaging mechanism to include source and manifests for extracted plugins. Replacing a bundled package with a user fork preserves its user documents but requires a new package-identity admission. A pack entry cannot whitelist private Rust APIs.

| Feature | V1 destination | Public services needed |
|---|---|---|
| Back burner | Bundled context service and command/panel plugin | Rooms, pane snapshots/events, cleanup operations, attention, conditional close/recall and undo |
| Quick Notes, notes picker, text/notes editor | Bundled notes plugin with capture panel and document panes | Text/document surfaces, scoped files, search, drop/clipboard references, trash/restore receipts, undo |
| Focus Writing | Bundled example plugin, independently removable | Document surface, settings, timers, scoped presentation lease; no private focus mutations |
| Themes | Bundled data packages, including light/dark variants | Theme validation, preview/apply and semantic roles; no runtime or capabilities |
| Needs you and Permissions/history UI | Bundled plugin surfaces | Scoped attention, verified grant/history projections, host-rendered consent and recovery |
| Agents app | Bundled dashboard plugin | Scoped agent/run projections and registered operations; no raw run/grant files |
| Secrets manager | Bundled settings UI | Redacted secret inventory and secure host edit/use controls; secret authority stays core |
| File browser | Bundled plugin | Folder binding, paginated listing/watch/search, document/file-handler commands; explicit optional cwd following |
| Image, audio and video viewers | Bundled viewer plugins, retaining existing experiment gates where applicable | Read handles and asynchronous media surfaces; codecs, devices and buffers remain host services |
| UI gallery | Bundled example plugin | Only the public semantic UI schema and diagnostics |
| Python apps already in the core pack | Same packages on the unified contract | Replace manifest-only admission; preserve their document formats and public effects |
| Agent light/dark synchronization | Optional bundled code plugin, off until configured | Theme-change event plus narrow supported harness-setting operations. A theme package itself cannot execute this hook. |
| Stint board | Separately installed reference consumer | Folder binding, 1175 typed operations, operation status and safe UI; task storage stays in the Stint repository |
| CLI/MCP renderer and command-view experiments | Optional plugins if retained by the V1 feature sweep | Descriptor data, typed operations and admitted connector services. Remove native workflow implementations at cutover; do not promote experiments merely to preserve them. |

Keep terminal/PTY ownership, layout, context identity, routing, generic editor/media rendering primitives, the command palette engine, broker, audit, trusted confirmations and secure inputs in core. Rich note semantics, cleanup policy, dashboard choices and presentation workflows move out. Hackability comes from exposing machine operations as typed capabilities with explicit admission and service boundaries. If a core plugin needs a missing operation, add it publicly with negative tests before extracting that feature.

## Proof specifications

Each proof runs through a bundled package and an independently installed copy using the same manifest/schema. Removing bundled provenance must not change the admitted behavior. Required checks follow [TESTING.md](../../src/testing/TESTING.md); the cases below are specifications, not reported test results.

### Back burner room

Use one room per context by default. Moving a pane removes its layout slot while retaining pane ID, process/session, terminal buffers, app state and runtime ownership. Keep servicing it; needs-you and failures remain visible. This copies the prior consult's separation of layout from lifetime and close-on-success/reveal-on-failure pattern. It replaces the consult's restore-to-slot choice with 1183's configurable split recall, and removes its compatibility aliases.

`backburner.send` captures selected panes, a room reference and a cleanup-policy revision. Policies select an ordered suite of declared typed operations by pane type, with agent-definition overrides and a conservative fallback. Bind the suite to the pane generation, agent run, folder and activity revision. Configuration cannot grant process execution. Use 1175's executable/argument admission; never inject cleanup strings into a PTY or invent an unrestricted shell executor. A user script can be a declared approved program with bounded argv, time and output. Missing policy or declined authority leaves the pane retained with an actionable request.

The host issues a cleanup lease. The plugin runs the suite asynchronously. All steps exiting zero requests conditional close against that lease; the host verifies the same pane, no intervening activity/dirty state and no unresolved blocking request, then closes and records the outcome. Nonzero, timeout, cancellation or failed preconditions produce one attention item with bounded output and Recall/Retry actions. New activity invalidates the lease, preventing a late zero from killing resumed work. Recalling cancels cleanup and makes stale completion inert.

Recall captures an explicit target pane/context and split policy at invocation. If that target disappears, return a visible target error and offer a new selection. Cross-context recall needs separate authority. Agent self-send is allowed only through the admitted public command for its own pane. A plugin crash leaves panes in the host room, listable and recoverable through core commands. Restart preserves room membership and saved launch/history records, but must not claim to revive a killed process or rerun destructive cleanup automatically.

An undo of Send to back burner recalls the still-live pane and cancels its cleanup lease. After a successful cleanup has closed the process, that inverse is unavailable. This resolves 1201's suggestion that close might park: a retained move is reversible; actual process termination remains irreversible and never restores the retired park API.

Core changes: pane storage independent of visible layout, room handles, revisioned events, cleanup leases and conditional close, public recall command, attention/status plumbing and undo contributions. Reproduce today's Cmd+U failure before replacing it, as 1183 requires. Prove zero-exit close, nonzero attention/recall, agent and ordinary-process cases, focus/target changes, late completion, plugin crash, shutdown and restart. Scenes cover an empty workspace with retained panes and a failed-cleanup card. Installed acceptance includes Cmd+U, split recall and continued terminal output while retained.

### Quick Notes migration

Move capture and picker workflow from [quick_note.rs](../../src/overlays/quick_note.rs), [notes_picker.rs](../../src/overlays/notes_picker.rs) and their `PlexiApp` fields into the notes plugin. Keep the public capture command and intended key behavior: Enter commits, Shift+Enter inserts a newline, Escape dismisses. The host panel owns focus, paste/drop routing and dismissal; neither pasted text nor drops may reach the terminal behind it.

The plugin creates Markdown documents and attachments through scoped file/document handles using the existing notes addresses. It must not choose a new storage tier simply because it has a new package ID. Commit uses a transaction receipt and idempotency key; failure retains the draft. Clipboard and dropped images arrive as bounded host resource references. The picker uses search/list and document-open APIs. Trash returns a resource-specific restore receipt; undo validates the destination and external changes before restoring. Keep local text undo distinct from note deletion history.

Core changes: public transient panels, input/drop routing, document editing, scoped list/search, transactional file/attachment writes and trash/restore handles; expose existing editor primitives rather than reimplementing them in Python. Prove paste selection replacement, attachment rejection, duplicate commit prevention, save failure recovery, same-root tier deduplication, changed context root and preservation of existing notes. Inspect normal/narrow/raised-scale capture and picker scenes, then drive an installed capture/save/reopen/trash/undo walk. Delete native workflow state only after the public route covers it.

### Focus Writing

Build this as a new plugin using the same document surface as Notes. Opening a file obtains a document reference with its exact folder and write grant; a scratch document uses declared user-data scope. The plugin supplies minimal controls, optional word/session goals and timers. A scoped `presentation.acquire` lease requests distraction-free presentation for its own surface. The host retains an exit command, trusted consent and accessibility. Lease release on crash/close/revocation restores presentation without restoring stale focus or overriding a layout change made after acquisition.

Saving uses versioned atomic document writes; an external edit produces a conflict and keeps the draft. Text undo is owned by the editor; workflow contributions can undo rename or presentation changes when their preconditions still hold. Timers advance from host timestamps and do not require a guest repaint loop.

Core changes: reusable document surface, revisioned writes/conflicts, scoped presentation lease and public timer/settings support. No Focus Writing special case in `PlexiApp`. Prove readonly denial, crash restoration, external edit conflict, offline use, local Cmd+Z priority, reduced motion and inactive-context timers. Installed evidence must show another terminal remains usable while the writer is busy or failed.

### Themes as data packages

A theme has tokens, semantic roles and optional light/dark variants. It has no executable, lifecycle hook or capabilities. Selecting it is a host command with reversible preview/apply. Validate every required role and state against final composited fills at load; reject invalid/unbounded data with named failing roles and keep the prior theme. The same public resolver protects custom themes and bundled presets. Contrast thresholds are defined in the UI contract above, not duplicated in theme code.

Core changes: data-only manifest validation, theme resource loading, contrast validation and preview rollback, public theme-change event and UI role exposure. Do not choose a new default theme in this architecture task. A separate code plugin handles agent theme synchronization and requests its own narrowly scoped authority. Prove no code entry or capability can be smuggled into a data package; invalid contrast, unknown required role and malformed values leave the prior theme intact. Inspect light/dark and interaction states at supported scales with reduced motion. Install the same valid data package through bundled and external paths without a runtime or consent prompt.

## V1 sequence and deletion boundaries

Land coherent slices with their own evidence; do not ship mixed authority as the finished platform. The release sweep still decides whether each optional experiment is retained. These are dependency decisions, not a progress checklist.

1. Freeze the reviewed contract and publish schema/error/conformance fixtures. Reconcile 1151, 1140, 1207 and 1201 against these identities and lifecycle rules. Keep their domain ownership. Establish the extension inventory and generated/public reference ownership.
2. Unify broker admission, cancellation and exact bindings across Python, legacy app paths and agent adapters. Retain 1175's executable safeguards. Delete the legacy store and migration readers, manifest-as-grant behavior, dual writes and independent prompt queues in this cutover. Preserve or visibly invalidate existing exact grants; no silent authority conversion.
3. Fix explicit context targeting and root readers, then expose folder bindings, Open in folder and shared state addressing. Correct the raw-WASI/component documentation distinction. Delete ambient active-context selection for plugin launches, installed-package-root overrides and native WASM's competing sanitized state-address scheme after explicit recovery of existing data. Preserve channel-neutral document addresses and 1179's separate secret migration.
4. Deliver supervised Python runtime/helpers and bounded host service work before admitting background plugins. Delete synchronous guest waits/joins and file effects from terminal/UI servicing paths. Keep native WASM experimental until equivalent isolation is qualified.
5. Expose commands, panels, room events, safe UI, operation status and trusted attention review. Reuse the completed native kit through 1207. Delete duplicate plugin/palette action metadata and separate consent presentation routes. Operation status and durable permission history belong in V1 alongside 1140; rich history export/search and general provider adapters may follow.
6. Extract Notes and prove Focus Writing and themes. Add undo contributions with 1201. Delete `PlexiApp` quick-note/picker workflow state, native overlay implementations and direct host-only note actions once equivalent public operations pass. Retain generic editing/file services. Theme activation cannot execute hooks.
7. Deliver 1183's back burner proof. Delete `hide_pane`, hidden placeholders, hide/reveal/unhide commands, park/unpark commands and corresponding bindings/config aliases after preserving recoverable pane/layout data. No compatibility aliases. A restore reader may recover old saved panes into explicit rooms once; it must not keep the hidden-pane execution model alive.
8. Extract the remaining chosen core-plugin workflows from `builtin_factory`, publish readable sources and validate independent installs/forks. Delete their native factory arms and private downcasts/call paths. Remove unretained experimental workflows instead of creating privileged exceptions. Verify pack membership, generated help and command coverage at the final combined head, then run the installed V1 feature sweep and Ian's acceptance.

A clean API break does not authorize deleting user documents. Legacy executable paths and authority fallback code go away; old portable data is preserved or moved aside with explicit, audited recovery where required. No ongoing dual writer or compatibility dispatcher survives.

## Appendix: draft implementation stint scopes

These scopes are review material only. Create no tasks from this draft until Ian approves. When filed through the create-stint workflow, each carries `v1`, links to 1182 and this document, names its dependency and applies TESTING.md. Existing owning stints receive scope refinements rather than duplicate tasks. Each behavior-changing slice includes an info trace with IDs/outcomes and no secret payloads.

| Draft scope / existing owner | Dependency | Done-When and qualification |
|---|---|---|
| Public package/contribution contract | Design approval | Manifest, PGAP, SDK and versioned WIT describe the same identity, operation/status and contribution types; unsupported versions fail clearly. Round-trip and malformed/oversized fixtures exercise both adapters. Generated references identify proposed versus shipped capabilities. |
| One admission authority | Contract | Production `PermissionStore` references, manifest grants and dual prompt paths are gone. Positive and negative fixtures cover each effect family, forged callers, changed package/resource, revocation, audit failure, once replay and delegation narrowing. Installed bundled/external parity and real trusted-consent controls pass. |
| Explicit folder bindings and targeting repairs | Contract and broker | Tests first reproduce invalid-target fallback, inactive-context launch and stale root reads. ID/path/local/global launch share one binding. Open in folder, retarget, restore and state address parity pass A/B-folder negative controls. Installed board walkthrough proves the actual folder even with remembered grants. |
| Supervised runtime and host work budgets | Contract | Python guest/helper lifecycle, queue limits, fair servicing and cancellation meet the isolation budget under loop/flood/close fixtures. Installed terminal latency and occlusion evidence pass; inline-call/join negative controls fail. Native WASM stays explicitly experimental unless qualified. |
| Commands and plugin contributions, refine 1151 | Contract and binding | Registry exposes public contributions, explicit targets and unavailable reasons; one handler serves palette/key/CLI routes. Forged approval and stale target attempts fail. Installed palette/keybinding walk and generated help agree. |
| Typed attention/status/history, refine 1140 plus companion scopes | Broker and contract | Source-owned actions, host consent cards, cancellation, scoped reveal, operation query/events and durable verified history agree across restart and missed events. Stale click, pane death and fake provider-resolution controls fail. Installed Stint Board wait/approve/deny/cancel walk passes. |
| Public safe UI, refine 1207 | Contract; native kit API settled | Host, bundled plugin and independent fixture use the same allocator/color resolver. Geometry, accessibility, action routing, limits and theme contrast pass through PGAP and WIT; narrow/scale/reduced-motion scenes are inspected. No reliance on another branch's private structs. |
| Secrets by reference, integrate 0788/1179 | Broker and contract | Reference use and secure inputs disclose no plaintext to guest/agent/logs. Revocation and channel ownership fail closed. Throwaway-keychain and separately signed installed checks preserve the distinction between PR file-backed keys and normal Keychain evidence. |
| Document services and Notes extraction | Binding, safe UI, runtime | Quick Notes proof passes with existing notes preserved; capture/picker/editor workflows use public calls and native workflow state is deleted. File conflicts, transaction retry, drops, clipboard, trash and recovery have negative controls and installed proof. |
| Undo contribution contract, refine 1201 | Broker, command registry, operations | Apply/undo/redo round-trip; stale preconditions, missing contributors, revoked grants and unknown outcomes do not move history incorrectly. Text Cmd+Z wins; acknowledged inverses produce named toasts. |
| Focus Writing example | Document services and safe UI | Its proof passes without a host special case; crash releases presentation and external edit keeps the draft. An independent install exercises the same APIs and terminal responsiveness. |
| Data theme packages | Package contract and color roles | Load validation, preview rollback and no-code/no-capability rejection pass. Bundled/external data packages behave identically; no default-theme taste decision is bundled into the change. |
| Back burner, refine 1183 | Rooms, status/attention, runtime, commands, undo | Existing failure reproduced first, then zero/nonzero cleanup, late completion, recall, process identity and restart proofs pass. Help/source coverage proves old hide/park paths removed; installed Ian acceptance remains explicit. |
| Remaining core-plugin extraction and public-only audit | Proven primitives and selected feature sweep | Agents, secret/settings UI, browser, viewers, gallery and retained renderer workflows pass domain-specific installed walks; pack packages have no private API imports, bundled bypass or native workflow factory arms. Disabled optional experiments remain disabled. |

## Decisions for Ian's review

In order of architectural impact:

1. Approve extending apps into one plugin model and requiring public-only bundled workflows, including File Browser and retained native renderer features.
2. Approve supervised Python helpers as a V1 prerequisite for the terminal-responsiveness promise, with the startup/memory cost measured during qualification. The alternative is a narrower promise with cancellable in-process workers, not claiming complete isolation.
3. Approve legacy-grant invalidation and reconsent where exact authority cannot be proved, while preserving user documents and separately audited secret recovery.
4. Approve per-context back burner rooms, conditional close on a successful cleanup suite and split recall; confirm this replaces the older restore-to-slot consult recommendation.
5. Approve the proposed V1 boundary for optional features and session-scoped undo history. 1201 can choose durable history later without changing the contribution envelope; theme choice remains a separate taste decision.

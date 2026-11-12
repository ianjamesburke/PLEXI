# Thin Plexi core and core plugins on one capability API

Status: active

Stint: 1182 (design); 1183 (back burner), 1140 (attention), 1151 (commands), 1207 (public UI), 1201 (undo). Additional execution scopes are drafts in the appendix.

## Decision

Install a **plugin**, open its **views**, and place a view in a host-owned **pane** or panel. Extend the existing package/runtime model with contributions, then replace its public app terminology. Keep one installer, manifest, identity, lifecycle, capability vocabulary, broker and distribution path. A plugin may contribute views, commands, panels, lifecycle subscriptions, themes, agent definitions or skills. Contributions acquire no authority merely by being installed.

Ian chose naming option A on 2026-11-10: **Plugins** is the public category, **Bundled** is the provenance badge, and **Marketplace** is the catalog. Everyday actions use feature names such as Open Notes and Choose theme. The `plugin` and `view` CLI namespaces replace `app` completely, without aliases.

The core owns terminal execution, pane identity and layout, input routing, rendering primitives, persistence transactions, resource scheduling, capability enforcement, trusted consent and the command registry. Bundled plugins own the replaceable workflows built from those services. They ship inside the Plexi distribution, with readable example source, and use exactly the public API available to an independently installed package. Bundling affects availability and provenance; it grants no permissions.

For V1, Python on Wasmtime is the sandbox for third-party code, running inside supervised helpers. Ian is the primary plugin developer. Release 0.5.0 supplies the smallest surface on which he can build real plugins, with Notes as the first bundled plugin and proof. Expand it incrementally according to the release sequence below. Native WASM components stay experimental and outside V1 product scope. Native dynamic-library plugins are excluded. Marketplace distribution and revenue remain Phase 4 work.

This document restores the path referenced by `src/broker/mod.rs` and replaces the deleted broker proposal from `7c62636c`. Its broker section supersedes the old migration/default-allow policy. It specifies the destination; it does not claim the API or isolation changes already exist. The task graph owns progress. Before closing the design task, link approved execution tasks here; delete this PRM only with its last execution task, after moving durable contracts beside their owners and repairing incoming links.

## Evidence and decisions from the design inputs

The inventory was checked against source commit `750eaa9677f8dca233175af149234431d8f4ad5f`. This is source evidence, not installed or measured runtime evidence. The relevant product direction is [NORTH_STAR.md](../../NORTH_STAR.md), particularly the V1 progression and commandments on commands, authority and ownership. The release owner's `TRUTH.md` places the plugin platform after the 0.4.4 patch snapshot; this design does not pull automatic ask reveal into that patch.

The supplied audits are local design evidence in the main checkout's git-ignored task store. Their recommendations are incorporated here; they are not prerequisites a plugin author must locate:

| Input | Decision carried into this contract |
|---|---|
| `.stint/runs/design-20261109/asks-ux-audit.md`, sections on operation state, typed cards and common projection; stint 1140 | One source-owned attention projection, host-owned consent cards, correlated operation status, atomic orphan cancellation and verified history. The inbox plugin presents requests; it cannot manufacture approvals. |
| `.stint/runs/design-20261109/context-roots-audit.md`, proposed per-app binding | Separate owning context, package directory, working folder and state address. Add explicit Open in folder and inspectable bindings. Shell `cd` proposes a folder; it never retargets a running plugin instance. |
| `.stint/runs/design-20261109/ui-layout-safety.md`, contract and migration; stint 1207 | Publish safe rows, action toasts, headers and surface color roles through both public transports. Reuse the host kit after its API settles; do not copy unmerged symbols from the 1202/1206/1203/1204/1205 lane. |
| Stint 1201 | Plugin undo contributions join host history through validated inverse operations. Text undo retains first refusal. Undo of a running pane's move is possible; revival of an exited process is not. |
| Stint 1183; stint 0896 and `.stint/plans/HIDE-PARK-CONSULT.md` | Adopt layout/lifetime separation and attention on failure. The newer back burner ruling replaces the consult's hide/unhide naming, aliases and restore-to-slot requirement. |
| Stints 1151 and 1175 | Use the shared command registry and existing typed CLI operation admission. Do not build a second dispatcher or an unrestricted shell escape. |
| Stints 0788 and 1179 | Keep secrets out of agent environments, preserve trusted host consent, and preserve channel-owned secret namespaces. Plain user documents remain channel-neutral. |
| `.stint/runs/design-20261109/naming-plugins-apps-views.md`, option A and naming rules; `.stint/runs/codex-lanes-20261109/briefs/astra-1182-revision-1.md`, Ian's rulings dated 2026-11-10 | Adopt plugin/view naming and the clean public rename. The later placement ruling broadens the audit's pane-only view definition: every view works in every supported slot. A panel is its host presentation, not a second rendering model. Add two-mode AI, concrete secret flows and the public developer test kit. Keep the name back burner. |

Load-bearing choices:

| Choice | Alternatives | Recommendation and reason |
|---|---|---|
| Extension identity | Separate extension system; extend the existing model and rename it plugin/view; native libraries | Extend and rename, as Ian ruled. A second system duplicates grants, packaging and UI. Native dynamic-library plugins are excluded. |
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
| [AI broker](../../src/plexi_ai/broker.rs): `ModelMode`, `backend_tier`, `ModelMode::from_wasm_tier`, `LiveAiBroker::dispatch`; [protocol](../../src/protocol/primitives.rs): `ModelTier`; [backends](../../src/plexi_ai/backend/mod.rs) | Configured fast/slow routes, request dispatch and host ledger | Public `Low/Medium/High` duplicates `Fast/Slow`; Medium currently maps to Fast. Only OpenRouter dispatches; other provider names return `unsupported_model_provider`. `backend/local.rs` and `backend/ollama.rs` exist but are not compiled by `backend/mod.rs`. |
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

The surface is versioned **v1**, but it is explicitly **not frozen until an outside author ships a plugin**. Ian’s bundled and independently installed proofs qualify the implementation; they do not meet that outside-author milestone. Before it, coordinate contract revisions with manifests, SDKs, fixtures and bundled consumers, reject incompatible requirements explicitly, and document breaking changes without compatibility aliases. At that milestone, freeze the demonstrated contract and use explicit versioning for subsequent incompatible changes. The release sequence defines when each destination API below becomes available; a v1 label does not promise every planned contribution in 0.5.0.

All API names below are destination names, not claims of shipped behavior. PGAP's typed schema, Python SDK and versioned WIT definitions describe the same operations, errors, identities and lifetimes; wire encodings may differ. Rename the existing installer and registry into the single `plugin` ecosystem. Do not add a parallel installer or JSON escape hatch.

### Naming and public entry points

| Object | Rule |
|---|---|
| Plugin | Installed identity, version, declared contributions and requested powers. Bundled is provenance. |
| Package | Artifact carrying a plugin. Distinguish stable plugin ID, version and verified content digest. |
| Instance | Host-managed active execution of a code plugin. It can exist without UI and own several open views. |
| View | Named UI contribution, such as `notes.editor`, with a default presentation. It can open in any supported host slot; multiple openings can show the same declared view. |
| Pane | Host-owned live container with layout, input and lifetime identity. It contains a view, terminal or Portal. Closing it does not uninstall a plugin or stop every instance. |
| Panel | Host-owned modal, sheet or sidebar presentation of a view, with focus, dismissal and bounds. A panel declaration references a view and its default presentation; it introduces no second UI callback or restrictive slot type. |
| Surface | Qualified technical term: document surface, input surface, GPU surface or trusted consent surface. Avoid it as the main user-facing category. |
| Component/widget | UI building block. Prefer component in SDK docs; widget is acceptable for native controls. Neither has package identity. |
| Theme | Color/role data selected by the host. A theme-only package executes no code. |
| Agent / run / harness / skill | Definition / execution / execution adapter / instructions. Plugins may distribute definitions and instructions; identities and grants stay separate. |

`plugin install/list/info/init/check/test/package/inspect` targets packages and their declared contributions. `view list/open/render` targets view declarations. `pane focus/close/action` targets a live pane. `plugin state` keeps explicit data scope; `plugin tools/call` carries an explicit instance where needed. Package lifecycle verbs such as uninstall/update/validate/freeze/prune also move to `plugin`. Existing domain commands such as `note` may remain registry-contributed actions. Retained CLI/MCP visualizers expose named views rather than `app open --cli/--mcp`.

`view open notes` resolves only a declared default or a uniquely declared view; otherwise it returns the choices. A theme-only plugin has no view to open. Local-path opening follows the same rule. Every opening returns an opening ID, view ID and instance association, plus a pane ID for pane placement or panel ID for panel placement. Machine output keeps `plugin_id`, instance, view and container identities separate.

Keep `plexi_sdk`, `init`, `update` and the rendering callback `view()`, with an explicit opening/view target. Rename `AppHarness` to `PluginHarness`, `AppBar` to `SurfaceHeader`, `AppPalette` to `PluginPalette`, `AppEvent` to `PluginEvent`, and package-identifying `app_id` to `plugin_id`. Keep host `PlexiApp`, qualified `AgentCard` and visual `Card`; tile remains internal. If retained, Command View becomes the Agents dashboard plugin's named view.

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

Replace public `AppManifest` with `PluginManifest`: `[plugin]` identity, `[plugin.capabilities]`, an explicit `[runtime]`, and `[[contributes.*]]` declarations. Keep plugin ID, version and verified content digest separate. The host derives the digest and provenance; a package cannot claim to be bundled. Contributions use plugin-namespaced stable IDs. Keep neutral filenames `manifest.toml`, `PACKAGE.toml` and `.plexipkg`; bump their schemas and reject old shapes without a dual parser.

| Manifest field | Meaning |
|---|---|
| API and runtime requirements | Required contract major, compatible schema revision and supported features. Reject unsupported mandatory features before activation with a typed error. Optional contributions may be unavailable with a reason. |
| Entry | Python entry for V1 code plugins; a data package has no entry or executable. Existing experimental WASM targets declare their world and feature requirements. |
| Contributions | Views, panel defaults referencing views, commands, settings, file handlers, subscriptions, undo handlers, themes, agent definitions and skills. Each declares scope, activation trigger and stable ID. |
| Capabilities | Requested actions and resource patterns, with human-readable reasons. These are ceilings on requests, never grant records. |
| State | Declared global/context scopes and portable format, through the existing user-data resolver. Document content uses document references, not an invented per-channel plugin directory. |
| Secrets and CLI operations | Secret names/purposes and typed operation declarations, admitted through their resource adapters. No plaintext credentials or arbitrary executable paths. |

Illustrative destination manifest shape:

```toml
[plugin]
id = "notes"
version = "0.1.0"
api_major = 1

[plugin.capabilities]
requests = ["notes.read", "notes.write"]

[runtime]
kind = "python"
entry = "main.py"

[[contributes.views]]
id = "notes.picker"
default_presentation = { slot = "modal", size = "medium" }

[[contributes.views]]
id = "notes.editor"
default_presentation = { slot = "pane", mode = "split", direction = "right", size = "large" }

[[contributes.commands]]
id = "notes.open_picker"
handler = "open_picker"
```

Panel declarations use `[[contributes.panels]]` with a view reference and default modal/sheet/sidebar presentation. Settings, subscriptions, undo handlers, themes, agents and skills use their own `[[contributes.*]]` tables. A theme-only manifest omits runtime/entry and capability requests. This sketch defines the object boundaries; the schema stint must define and validate each field and capability ID together.

Use a host-managed instance lifecycle: registered, activating, ready, stopping, stopped or failed. Activation happens on a command, surface open or admitted subscription, rather than eagerly starting every installed package. A theme never starts a runtime. A context service instance can own several transient surfaces; closing its panel need not end its service. The host enforces instance and resource quotas. Package disable/uninstall stops instances, cancels operations, unregisters contributions and retains user documents unless the user separately deletes them.

The host sends an initialization snapshot containing instance identity, owning context, folder/state bindings, admitted handles, config revision and available API features. Updates deliver typed events; views return declarative trees; effects request operations. Shutdown is advisory to the guest and bounded by the supervisor. A new instance generation invalidates old actions and handles. Upgrade quiesces the old generation, saves portable state, swaps atomically, and re-admits changed authority. A failed upgrade leaves the prior package available; it never silently broadens its grants.

### Identity, scopes and folders

**Agents are not plugins.** An agent is a principal that acts on Plexi; a plugin extends Plexi. The agent definition, its authenticated run and its harness retain the meanings in `src/agent/AGENTS.md`. A plugin can ship agent definitions and skills, but installing it neither runs them nor combines their identities or grants. Instructions grant no authority.

Every request carries a host-stamped caller principal and generation. A plugin-supplied context, pane or folder is a target to resolve and authorize, never caller identity. Context ownership and cross-context reach follow [the host scope contract](../../src/host/AGENTS.md). Package provenance is a separate field, never a scope or trust shortcut.

Extend the host origin model to identify context-service instances without inventing a hidden pane to hold their authority. Pane identity is optional for such instances; authenticated instance identity is mandatory. When an agent or another plugin invokes a contributed command, carry its delegation chain and admit the intersection of caller delegation, plugin ceiling and resource grants. A plugin's remembered grant must not become an ambient privilege the invoking agent can borrow. Host-created scheduled work uses its own explicitly approved activation scope.

The host issues an `InstanceBinding` with `instance_ref`, `context_ref`, `folder_ref`, `state_scope` and `binding_revision`. `folder_ref` is a canonical resource reference owned by the binding; it does not grant filesystem or CLI access. State addresses continue to use the channel-neutral resolver. Runtime reach uses context IDs even when two contexts have the same folder. Config discovery continues to shadow by registry source.

Global state is still plugin-owned state, not permission to read every plugin's files. Shared collections such as the existing notes tiers require an explicit collection grant. Keep structured plugin-to-plugin events on the event bus or directed JSON pipes and bulk media on typed pipes; pane lifecycle subscriptions do not create a fourth data transport.

`view.open` takes a view reference, explicit owning context, optional working folder and placement. The folder default is that context's root, independent of focus and package installation location. CLI `view open <id> --folder <path>` and the palette's Open in folder invoke this operation. `--folder .` deliberately captures CLI cwd. Instance reuse matches plugin, context and folder; opening reuse also matches the view and requested presentation. An A-bound instance cannot satisfy a request for B.

`binding.inspect` is available to the instance and authorized observers. Host view chrome always shows the working folder; structured pane or panel state exposes it and the state scope. `binding.retarget` shows old/new targets, stops new admissions under the old revision, cancels pending approvals and creates new requests only after revalidation. Running operations retain their original target and visible status. Old results cannot update a newly bound document. A root change emits a revisioned scope event and cancels stale state writes; it does not move a running shell or silently copy data. Restoring a missing folder reports an error, with an explicit correction flow, rather than falling back to home.

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

Retire `PermissionStore`, dual writes, Python manifest-as-grant checks and runtime-specific approval queues. Preserve independently verified exact broker grants; invalidate legacy records that cannot prove the new binding, retain an inert export for inspection, and ask again. Do not ship a fallback reader or compatibility grant path. Preserve user documents and separately audited secret recovery; 1179 owns secret migration. Ian settled this invalidation and re-request policy in revision 2. Data recovery must never reactivate an unverifiable grant.

### Operation state and cancellation

An effect submission receives an `operation_ref`. `operation.status` and ordered status events report `queued`, `waiting_for_approval`, `running` and a terminal result (`succeeded`, `failed`, `denied`, `cancelled`, `expired`, or `outcome_unknown`). Include request correlation, sequence, binding revision and an optional attention reference. Status queries reconcile missed events; delivery is replayable, execution is deduplicated. Approval alone never produces success.

Cancel, timeout, pane death, instance death and plugin disable share one owner transition that retires pending authority and the continuation before acknowledging cancellation. A late click is rejected. If a native program has already made changes, report cancellation with possible partial effects instead of promising rollback. The host supplies a waiting strip even if the guest ignores status events; it disables only the affected action and leaves other controls usable. External drains run in `App::logic`, with inactive-context servicing through `background_tick` or its successor.

### Secret references and use cases

`secret.resolve(name, purpose)` yields an opaque `SecretRef` bound to principal, resource, channel, binding revision, secret version and intended use. `secret.use` substitutes it only into an admitted host-owned operation. Secret inventory and edit controls are host-owned; a secure input field never returns its value to the plugin. Any exceptional raw read retains a distinct broker action and trusted disclosure. Follow 1179's channel ownership; do not search other channels when a secret is absent.

| Case | Public flow and authority |
|---|---|
| Plugin needs an API key | Its manifest declares the canonical name and purpose. It requests a reference; the host resolves the selected binding, asks for scoped consent or a missing value through secure UI, then uses the reference in an admitted HTTP/AI request. Credential injection binds to the approved endpoint; redirects cannot forward authorization to another origin. The plugin receives status/data, never the key. |
| Same name, different value per context or folder | Host config maps the canonical name to a secret resource for the explicit context/workspace binding. A folder override, when enabled, resolves against the instance's canonical working folder. Switching bindings cancels pending use and requires a newly resolved/admitted reference; a parent/home grant does not authorize the new value. Show the selected source and missing/locked errors without exposing bytes. An explicit configured fallback is disclosed and admitted, never inferred from current focus. |
| Environment variable for an approved CLI operation | The operation declaration maps an allowlisted environment variable to a secret name and purpose. The host admits secret use together with the exact 1175 program, definition, workspace and package binding, then supplies the value only to that invocation's environment. The guest supplies neither arbitrary environment entries nor plaintext. Changed mappings require reapproval. Never put it in argv, an agent pane environment or the helper's environment. |
| Rotation | A host-owned secure edit updates the owning channel's value in place and increments its version. New uses resolve the new version; pending old references are cancelled or re-admitted. Running native operations retain the version already delivered until cancellation/exit; rotation cannot retract bytes from a process. Log the resource/version/outcome, never the value. |

Current source boundaries: `workspace::secrets::resolver` owns canonical-name routing; `release::catalog::ReleaseFeature::FolderSecrets` gates `folder_secrets`, which is off by default. `cli_ops::exec::fixed_env` and its cleared launch environment do not yet provide the proposed secret mapping. Context/folder resolution and explicit CLI injection must be qualified as public operations before being advertised. This design does not silently graduate the folder experiment or enable automatic terminal injection.

An approved native program receives the secret and has account authority. It could expose the value in output or elsewhere; the consent must disclose that recipient. Redact known secret values from host diagnostics and returned output, but do not claim that redaction confines arbitrary native code. No general credential proxy is added.

### AI: fast/slow and OpenAI-compatible endpoints

Plugins request `ai.query` with exactly two modes, `fast` and `slow`, matching `[models.fast]` and `[models.slow]`. They supply messages, typed tools and request limits, not provider credentials. The host resolves the mode from the calling instance's explicit workspace/context config, captures the route/config revision for that operation, admits it and streams a correlated result. Config changes affect subsequent requests; they do not silently retarget in-flight work. Agent-selected routes remain separately admitted and cannot widen a plugin's request authority.

Current duplication lives in `protocol::ModelTier::{Low, Medium, High}`, `plexi_ai::broker::ModelMode::{Fast, Slow}`, `backend_tier` and `ModelMode::from_wasm_tier`; the latter maps both Low and Medium to Fast. Replace the public tier shape with Fast/Slow across PGAP, SDK and WIT. Delete Medium and the legacy low/medium/high request spellings and conversion paths without aliases. Provider reasoning-effort values, including `ReasoningEffort::Medium`, are a separate setting and are not removed by this mode decision.

Use one provider type: **OpenAI-compatible endpoint**, configured by base URL plus secret name. OpenRouter is the default preset of that type, not a privileged dispatch branch. The fast/slow config selects a named endpoint and model; workspace/context overrides may choose different routes. The host resolves the endpoint's secret reference for the request, keeps the key outside plugins and agents, and validates destination changes before reuse of credential authority. Local endpoints use the same type when they implement the required protocol; unsupported streaming/tool features fail with typed errors rather than a silent provider fallback.

`LiveAiBroker::dispatch` currently selects only `openrouter` and returns `unsupported_model_provider` for other names. `backend/mod.rs` compiles only `openrouter`; the present `local.rs` and `ollama.rs` files do not establish working providers. Replace this switch with the common endpoint client and remove dead provider-specific paths. Validate URL, credential handling, streaming, tool calls, cancellation, usage and errors through protocol fixtures before claiming an endpoint supported.

Spend caps, request admission and the ledger stay in the host. Bind ledger entries to plugin/agent principal, context, resolved endpoint/model and operation, without keys or prompt contents. Enforce aggregate reservations before dispatch so concurrent requests cannot each spend the same remaining budget. Preserve missing usage/cost as unknown; it must not become free usage. A configured cap with unavailable accounting fails visibly rather than reproducing `LiveAiBroker::dispatch`'s current fail-open ledger read. Provider-reported usage reconciles the reservation when available. Changing mode or routing does not reset caps or erase history.

### Attention return

The attention API exposes a versioned projection with `attention_ref`, source owner, host-bound origin, kind, state, permitted actions, disclosure and delivery receipt. Plugins can publish scoped questions, notices and failures, inspect admitted projections, request review and cancel their own work. Only the broker publishes Plexi grant actions. A supported provider adapter owns provider prompts and acknowledges actual delivery; closing an inbox item is not a provider response.

`attention.review` opens a host-owned consent surface by reference. The host renders identity, concrete target/arguments, workspace, exact remember scope/duration and native-program authority from validated records. Plugin explanations are attributed text. Detailed package/program hashes and resource declarations remain inspectable. Plugins cannot supply consent button callbacks or desktop decision tokens.

The bundled Needs you plugin implements 1140's scoped reveal: eligible requests in the foreground context reveal without stealing pane typing; other contexts receive quiet markers. Minimize lasts for the visit, and entering a context can reveal unresolved requests according to the setting. Global interruption needs a separate grant. A minimal core recovery command can open pending trusted consent and disable a failed inbox plugin. It contains no alternative workflow policy. Verified, paginated `permission.history` exposes consent/use/outcome after once grants disappear, without returning active authority or recreating grants.

### Commands, view placement, safe UI and pane hooks

Plugin commands register with 1151's registry: stable ID, label, aliases, argument schema, target requirements, availability reason, capability requirements and handler. Keybindings, palette, CLI and agent calls invoke that record. Capture invoking context, pane selection and window before opening a picker; revalidate on dispatch without substituting current focus. Unavailable commands remain discoverable. A command listing conveys no authority, and protected approval is an explicit non-dispatchable exception. The back burner cutover removes hide/park aliases even though 1151 otherwise preserves unrelated binding compatibility; its release is specified below.

#### View placement

A view declares a default presentation. Placement belongs to opening it, not to its type. The host owns this closed set:

| Slot | Opening options |
|---|---|
| Pane | Split with explicit direction and target, tab in a captured target, or a new context with an explicit root |
| Centered modal | Host-centered panel with focus ownership and dismissal |
| Sheet | Host-attached panel with explicit owner and dismissal |
| Sidebar | Host-reserved side region with explicit owner and focus behavior |

Toasts and attention are notification components, not view slots. Plugin requests cannot add arbitrary placement types. Every view renders correctly in every slot through host safe rows, reflow and accessible overflow. A plugin may choose a default but cannot reject a valid user-selected slot as the wrong view type.

Sizes are `small`, `medium`, `large` and `fit-content`. The host maps these to available window size, safe areas and UI scale; plugins specify no pixel dimensions. The host remembers the last slot and size per user, view and owning context, clamping restored geometry to the current window. Resolution order is explicit command/keybinding invocation, per-view user config, remembered presentation, then manifest default. Users can override a view through keybindings or config. For example, `view open notes.editor --as tab` overrides its right-split default; its target is captured before dispatch, not looked up from later focus.

Cmd+O invokes registered command `notes.open_picker`, which opens `notes.picker`, defaulting to a centered modal of size medium. Picking a document opens `notes.editor`, defaulting to a right split. The same picker must work as a tab or sidebar when configured that way. Closing or cancelling restores the prior live focus target; if it has disappeared, the host selects a valid fallback in the owning context and reports it. Completing a picker that opens an editor deliberately focuses that new opening. Remembered placement confers no authority to create another context or access another folder.

The host owns bounds, focus-stack participation, Escape behavior, safe areas and focus restoration. A plugin receives typed actions, paste/drop resource references and values within its view. It cannot draw over trusted chrome or intercept other panes' input. A stalled plugin cannot prevent the host from dismissing its view. Changing slots preserves the opening's document state and instance association; host-owned notifications remain outside that placement transition.

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

### Public developer test kit

Ship `PluginHarness` on the existing `HostHarness`, `PlexiUiHarness` and TOML scene infrastructure. It runs the production schema validators, host renderer, layout allocator and event routing. Do not create a separate renderer or simulated layout engine that can pass while the real host fails.

`plugin check/test` validates the package and exercises every declared view in every slot and size class, including pane split directions, tab and new-context placement. `view render` produces a headless image plus semantic/layout diagnostics for a named view, slot, size class, theme and scale. Fixtures seed document data, window dimensions and captured targets without requiring network credentials or touching user files.

The public harness scripts events and actions, config/placement overrides, focus, paste/drop handles, resize, cancellation and lifecycle changes. Fake broker decisions cover allow/deny/ask, revocation, expiry and failed audit/persistence; injection is test-only and cannot become a live approval endpoint. Assert attention kind/actions/status, delivery receipts, local text undo, scoped restore receipts and named toast outcomes. Add shared undo/redo history and room-event assertions with their later APIs; the test kit itself ships with the initial surface. Screenshots retain fixture, package digest, host head, placement, scale and theme metadata for inspection.

Conformance includes long content, narrow windows, changed scale, reduced motion, unreachable targets and crashed instances. Negative controls must catch a blank toast action, stale-target dispatch and a view hard-coded to modal geometry. Supply bundled and independent-plugin examples. These headless checks are V1 developer quality of life and pre-push evidence; they do not substitute for installed consent, native focus or terminal-responsiveness validation.

### Undo contribution point

1201 owns the history engine. A plugin contributes an `UndoEntry` only after a successful operation, containing a plain description, context, originating operation, resource versions, inverse/redo handler IDs, bounded opaque payload and expiry. The host issues the entry ID; the plugin cannot edit another contributor's history.

Undo/redo validates both resource preconditions and current capability grants, invokes the contributor asynchronously and moves the history cursor only after confirmed success. A stale file, missing plugin, closed process or revoked grant leaves the entry visibly unavailable. No success toast appears before acknowledgement; success says “Undid: <action>”. Crash during an external inverse can yield `outcome_unknown`; do not blindly replay it. New work after undo clears the redo branch. Use bounded, context-owned history with a global fallback for genuinely global operations; 1201 owns the history cap and restart-persistence decision. Neither is an open choice in this design. Cmd+Z goes to a focused editing surface first. Document-local edit history stays with that surface; plugin contributions cover workflow operations such as trash, rename or pane moves.

## Performance isolation required for V1

A supervised helper hosts each active code-plugin instance's Wasmtime store and decoder. It receives a narrowly authenticated IPC channel, read-only package/runtime material and explicit handles, with secrets and host socket authority removed from its environment. It cannot directly mutate host state. The helper remains an internal runtime mechanism behind the existing lifecycle; it is not a second plugin contract or a new agent sandbox.

The IPC boundary carries the same typed lifecycle, resource references, requests and results whether the runner is a local helper or, later, a remote or containerized runner. Remote authentication, transport, resource mapping and deployment are future work, outside V1. Keeping that contract portable does not claim those runners exist or give them host filesystem access.

The host never waits for guest work, helper shutdown, compilation, file/network I/O or decoding on the UI/terminal servicing path. Requests enter bounded queues; results return by nonblocking drains with per-instance and total work budgets. Coalesce view updates to the newest complete revision, preserve ordered control/results, and reject overload explicitly. Apply CPU/memory/IPC quotas and admission limits so many well-formed plugins cannot exhaust the host by multiplying their individual budgets. Use fair scheduling with reserved terminal service capacity. Cancellation and helper reaping run outside the UI path; a watchdog kills a stuck helper and leaves a host-owned failure card and recovery commands.

Host effects need their own isolation. Blocking filesystem, executable hashing, secret store calls and media work run on bounded service workers, with cancellable operation records. Bound tree validation, layout and painting as well as guest execution. A small JSON tree containing enormous text or media must not defeat the host budget. Repaint pacing follows existing visibility and reduced-motion rules. Warm pools and content-addressed compile caches may reduce startup cost after measurement; they cannot share plugin memory or authority.

Native WASM remains experimental and outside V1 product scope. Any future promotion requires this scheduling boundary, including link-time interfaces and audio/GPU work. V1 Python extraction must not wait for new non-Python runtimes. The contract and conformance fixtures reserve the same lifecycle/effects across both; experimental native calls must never be marketed as equivalent performance isolation while they remain inline.

Qualification must measure terminal input-to-paint latency and PTY drain progress beside a guest that loops, blocks a host effect, floods output, sends maximum-sized UI and ignores shutdown, including simultaneous instances, inactive contexts and an occluded host. The proposed acceptance budget is no guest-attributable host service slice above 2 ms and terminal p99 input-to-paint regression no greater than one display frame versus the same fixture without plugins. Record machine, display cadence, workload and distributions; calibrate a stricter supported envelope if needed, rather than accepting a failed budget. Close/disable must finish within the supervisor's declared deadline. Negative controls restoring inline lifecycle calls and synchronous joins must fail. This bounds plugin-caused stalls under supported load; it does not promise real-time behavior under arbitrary OS resource exhaustion.

## Bundled core plugins and the core boundary

The release sequence stages these destination workflows; this inventory does not require all of them in 0.5.0. The maintained membership source remains [packs/core.toml](../../packs/core.toml). Extend its packaging mechanism to include source and manifests for extracted plugins. Replacing a bundled package with a user fork preserves its user documents but requires a new package-identity admission. A pack entry cannot whitelist private Rust APIs.

| Feature | Destination plugin | Public services needed |
|---|---|---|
| Back burner | Bundled context service and command/panel plugin | Rooms, pane snapshots/events, cleanup operations, attention, conditional close/recall and undo |
| Quick Notes, notes picker, text/notes editor | Bundled notes plugin with capture panel and document panes | Text/document surfaces, scoped files, search, drop/clipboard references, trash/restore receipts, undo |
| Focus Writing | Bundled example plugin, independently removable | Document surface, settings, timers, scoped presentation lease; no private focus mutations |
| Themes | Bundled data packages, including light/dark variants | Theme validation, preview/apply and semantic roles; no runtime or capabilities |
| Needs you and Permissions/history UI | Bundled plugin surfaces | Scoped attention, verified grant/history projections, host-rendered consent and recovery |
| Agents dashboard | Bundled plugin with dashboard views | Scoped agent/run projections and registered operations; no raw run/grant files |
| Secrets manager | Bundled settings UI | Redacted secret inventory and secure host edit/use controls; secret authority stays core |
| File browser | Bundled plugin | Folder binding, paginated listing/watch/search, document/file-handler commands; explicit optional cwd following |
| Image, audio and video viewers | Bundled viewer plugins, retaining existing experiment gates where applicable | Read handles and asynchronous media surfaces; codecs, devices and buffers remain host services |
| UI gallery | Bundled example plugin | Only the public semantic UI schema and diagnostics |
| Python apps already in the core pack | Same packages on the unified contract | Replace manifest-only admission; preserve their document formats and public effects |
| Agent light/dark synchronization | Optional bundled code plugin, off until configured | Theme-change event plus narrow supported harness-setting operations. A theme package itself cannot execute this hook. |
| Stint board | Separately installed reference consumer | Folder binding, 1175 typed operations, operation status and safe UI; task storage stays in the Stint repository |
| CLI/MCP renderer and command-view experiments | Optional plugins if retained by the V1 feature sweep | Descriptor data, typed operations and admitted connector services. Remove native workflow implementations at cutover; do not promote experiments merely to preserve them. |

Keep terminal/PTY ownership, layout, context identity, routing, generic editor/media rendering primitives, the command palette engine, broker, audit, trusted confirmations and secure inputs in core. Rich note semantics, cleanup policy, dashboard choices and presentation workflows move out. Hackability comes from exposing machine operations as typed capabilities with explicit admission and service boundaries. If a core plugin needs a missing operation, add it publicly with negative tests before extracting that feature.

## Host projections

Release label: **0.5.0** defines the family and reserves the IDs; the first consumer, AI usage (stints 1229 and 1225), lands after 0.5.0.

A host projection is a host-owned typed stream a plugin may read but never publish. Each has a reserved publisher, a snapshot plus a cursor for resuming, and read grants admitted by the one broker like any other capability. It is modelled on the pane lifecycle projection (`src/host/pane_lifecycle.rs`: publisher `plexi.host.panes`, stream `pane.lifecycle`), which today is an in-memory event timeline without a snapshot read; the shared snapshot-plus-cursor shape is new and defined by the schema stint. Data is redacted at the host before it is published, so a reader never holds anything it could not safely display.

Reserved capability IDs: `usage.read` and `usage.refresh`. The schema stint must still define and validate capability IDs, and rejects a request for a reserved ID that no projection serves. The flagship projection reader is the AI usage plugin: the host owns the usage stream and adapters, and the plugin only renders it. A plugin without the grant gets a typed denial, and an independently installed copy behaves exactly like the bundled one.

## Plugin taxonomy by authority

Release label: **0.5.0** (rule and table); individual plugins land per the release sequence.

| Kind | Authority needed | Examples |
|---|---|---|
| View-only renderers | Safe UI and placement only | Themes, UI gallery, Focus Writing presentation |
| Projection readers | Read grants on host projections | AI usage view, Agents dashboard |
| Automation plugins acting through the broker | Per-operation broker admission | Routines, Stint board, light/dark sync |
| Harness and provider adapters | Credentials or long-lived vendor processes | Core only; never a plugin |

Deciding rule: anything that bears credentials or spawns a process as the user is host-owned, or brokered per operation. Rendering and policy are plugin-owned. Third-party plugins must never read `~/.claude`, `~/.codex` or `~/.pi`, call vendor endpoints, or see an unredacted account label.

## Agent hooks and state placement

Release label: **0.5.0**.

- Agent hook installers and the hook-event to agent-state translation table (`src/cli/agent.rs`, for example `map_hook_payload` and the Claude settings overlay) move to a bundled plugin. The `set_agent_state` protocol and host process/PTY observation stay core.
- The pane `state` and `status` commands merge into one command that carries provenance on every value: `reported` (hook or agent claim), `observed` (host process/PTY detection) or `inferred`.
- A harness with no hooks is shown as "observed only". It is never shown as "exited without report"; that state belongs only to supervised workers that owe a result.

## Routines and the scheduler

Release label: **0.5.x** for the plugin; the manifest reservation and the core-only item are **0.5.0**.

- The scheduler for routines is a core primitive with its own approved activation scope. Routines become a bundled plugin on that primitive in 0.5.x. Stint 1219 already gated the scheduler on the Routines experiment.
- Reserve `[[contributes.commands]]` and CLI namespace contribution in the schema-2 manifest now (0.5.0): host-executed typed commands, core namespaces reserved, first install wins between plugins, consent at install. A plugin never supplies a shell command line for the host to run.
- 0.5.0 core-only checklist item: core-only mode must also stop the scheduler.

## Proof specifications

Each proof runs through a bundled package and an independently installed copy using the same manifest/schema. Removing bundled provenance must not change the admitted behavior. Required checks follow [TESTING.md](../../src/testing/TESTING.md); the cases below are specifications, not reported test results.

### Back burner room

Use per-context rooms, as Ian ruled in revision 2. Moving a pane removes its layout slot while retaining pane ID, process/session, terminal buffers, plugin state and runtime ownership. Keep servicing it; needs-you and failures remain visible. This copies the prior consult's separation of layout from lifetime and close-on-success/reveal-on-failure pattern. It replaces the consult's restore-to-slot choice with 1183's configurable split recall, and removes its compatibility aliases. The public name remains back burner.

`backburner.send` captures selected panes, a room reference and a cleanup-policy revision. Policies select an ordered suite of declared typed operations by pane type, with agent-definition overrides and a conservative fallback. Bind the suite to the pane generation, agent run, folder and activity revision. Configuration cannot grant process execution. Use 1175's executable/argument admission; never inject cleanup strings into a PTY or invent an unrestricted shell executor. A user script can be a declared approved program with bounded argv, time and output. Missing policy or declined authority leaves the pane retained with an actionable request.

The host issues a cleanup lease. The plugin runs the suite asynchronously. All steps exiting zero requests conditional close against that lease; the host verifies the same pane, no intervening activity/dirty state and no unresolved blocking request, then closes and records the outcome. Nonzero, timeout, cancellation or failed preconditions produce one attention item with bounded output and Recall/Retry actions. New activity invalidates the lease, preventing a late zero from killing resumed work. Recalling cancels cleanup and makes stale completion inert.

Recall captures an explicit target pane/context and split policy at invocation. If that target disappears, return a visible target error and offer a new selection. Cross-context recall needs separate authority. Agent self-send is allowed only through the admitted public command for its own pane. A plugin crash leaves panes in the host room, listable and recoverable through core commands. Restart preserves room membership and saved launch/history records, but must not claim to revive a killed process or rerun destructive cleanup automatically.

An undo of Send to back burner recalls the still-live pane and cancels its cleanup lease. After a successful cleanup has closed the process, that inverse is unavailable. This resolves 1201's suggestion that close might park: a retained move is reversible; actual process termination remains irreversible and never restores the retired park API.

Core changes: pane storage independent of visible layout, room handles, revisioned events, cleanup leases and conditional close, public recall command, attention/status plumbing and undo contributions. Reproduce today's Cmd+U failure before replacing it, as 1183 requires. Prove zero-exit close, nonzero attention/recall, agent and ordinary-process cases, focus/target changes, late completion, plugin crash, shutdown and restart. Scenes cover an empty workspace with retained panes and a failed-cleanup card. Installed acceptance includes Cmd+U, split recall and continued terminal output while retained.

### Quick Notes migration

Move capture and picker workflow from [quick_note.rs](../../src/overlays/quick_note.rs), [notes_picker.rs](../../src/overlays/notes_picker.rs) and their `PlexiApp` fields into the notes plugin. Keep the public capture command and intended key behavior: Enter commits, Shift+Enter inserts a newline, Escape dismisses. The host panel owns focus, paste/drop routing and dismissal; neither pasted text nor drops may reach the terminal behind it.

The plugin creates Markdown documents and attachments through scoped file/document handles using the existing notes addresses. It must not choose a new storage tier simply because it has a new package ID. Commit uses a transaction receipt and idempotency key; failure retains the draft. Clipboard and dropped images arrive as bounded host resource references. The picker uses search/list and document-open APIs. Trash returns a resource-specific restore receipt; undo validates the destination and external changes before restoring. Keep local text undo distinct from note deletion recovery. The initial Notes plugin offers its own bounded Restore action using that receipt and rechecks grants and resource versions. This is the minimum recovery primitive required in 0.5.0; it neither introduces a shared history engine nor chooses its cap or restart persistence. Integrate the same operation into shared undo contributions when 1201 supplies them.

Core changes: public transient panels, input/drop routing, document editing, scoped list/search, transactional file/attachment writes and trash/restore handles; expose existing editor primitives rather than reimplementing them in Python. Prove paste selection replacement, attachment rejection, duplicate commit prevention, save failure recovery, same-root tier deduplication, changed context root and preservation of existing notes. Inspect normal/narrow/raised-scale capture and picker scenes, then drive an installed capture/save/reopen/trash/undo walk. Delete native workflow state only after the public route covers it.

### Focus Writing

Build this as a new plugin using the same document surface as Notes. Opening a file obtains a document reference with its exact folder and write grant; a scratch document uses declared user-data scope. The plugin supplies minimal controls, optional word/session goals and timers. A scoped `presentation.acquire` lease requests distraction-free presentation for its own surface. The host retains an exit command, trusted consent and accessibility. Lease release on crash/close/revocation restores presentation without restoring stale focus or overriding a layout change made after acquisition.

Saving uses versioned atomic document writes; an external edit produces a conflict and keeps the draft. Text undo is owned by the editor; workflow contributions can undo rename or presentation changes when their preconditions still hold. Timers advance from host timestamps and do not require a guest repaint loop.

Core changes: reusable document surface, revisioned writes/conflicts, scoped presentation lease and public timer/settings support. No Focus Writing special case in `PlexiApp`. Prove readonly denial, crash restoration, external edit conflict, offline use, local Cmd+Z priority, reduced motion and inactive-context timers. Installed evidence must show another terminal remains usable while the writer is busy or failed.

### Themes as data packages

A theme has tokens, semantic roles and optional light/dark variants. It has no executable, lifecycle hook or capabilities. Selecting it is a host command with reversible preview/apply. Validate every required role and state against final composited fills at load; reject invalid/unbounded data with named failing roles and keep the prior theme. The same public resolver protects custom themes and bundled presets. Contrast thresholds are defined in the UI contract above, not duplicated in theme code.

Core changes: data-only manifest validation, theme resource loading, contrast validation and preview rollback, public theme-change event and UI role exposure. Do not choose a new default theme in this architecture task. A separate code plugin handles agent theme synchronization and requests its own narrowly scoped authority. Prove no code entry or capability can be smuggled into a data package; invalid contrast, unknown required role and malformed values leave the prior theme intact. Inspect light/dark and interaction states at supported scales with reduced motion. Install the same valid data package through bundled and external paths without a runtime or consent prompt.

## Release sequence and deletion boundaries

Land coherent slices with their own evidence. Release 0.5.0 is the minimal usable plugin platform and Notes proof; later releases expand the same versioned v1 surface. The API freeze follows the outside-author milestone defined in Public contract, not the 0.5.0 tag. The order below is recommended implementation order, with hard dependencies stated explicitly. These are release dependencies, not progress records or promised patch numbers.

### Release 0.5.0: platform and Notes

1. **0.5.0: plugin/view contract and rename.** Publish schema/error/conformance fixtures and the initial supported-feature set. Reconcile 1151, 1140, 1207 and 1201 identities without requiring every later feature. Apply the plugin/view boundaries below across installer, CLI, SDK, manifest and public references as one schema cutover. Unsupported later contributions fail with a named reason before activation. Keep each domain's ownership; this is a versioned cutover, not an API freeze.
2. **0.5.0: one broker and legacy store deletion.** Depends on authenticated contract identities. Unify admission, cancellation and exact bindings across Python, retained native workflows and agent adapters; retain 1175's executable safeguards. Delete the legacy store and migration readers, manifest-as-grant behavior, dual writes and independent prompt queues. Preserve verified exact grants; invalidate and re-request those that cannot prove exact authority. Preserve user documents and audited secret recovery. No remaining workflow may keep a private admission bypass merely because its extraction is later.
3. **0.5.0: explicit context targeting and folder bindings.** Depends on the contract and broker. Fix root readers, expose Open in folder, retarget/restore and shared state addressing. Delete ambient active-context selection for plugin launches and installed-package-root overrides. Preserve channel-neutral document addresses and 1179's separate secret ownership. Correct raw-WASI/component documentation; removal of native WASM's competing sanitized state-address scheme requires explicit data recovery when that experimental adapter adopts the contract, outside the 0.5.0 prerequisite set.
4. **0.5.0: supervised Python helpers and bounded host service work.** Depends on lifecycle/request identities and admitted resource handles. Deliver helper isolation before running extracted code plugins. Delete synchronous guest waits/joins and file effects from terminal/UI servicing paths; bound queues, validation, rendering and service workers. Native WASM remains experimental and outside V1 product scope; remote/container runners remain future work.
5. **0.5.0: commands, view placement, safe UI and public PluginHarness.** Depends on the contract, explicit bindings, broker and runtime boundary; reuse the native kit through 1207 once its API settles. Expose every declared view in every supported slot and size class, shared command dispatch and the public host-based test kit. Delete duplicate plugin/palette metadata and consent decision routes. **Hard prerequisite pulled forward from attention work:** trusted host review, minimal recovery access, correlated operation status, waiting UI, atomic cancellation and durable redacted broker audit must ship here for safe Notes file requests. Full Needs you extraction, rich reveal policy, room events and permission-history browsing are later. Basic view/pane lifecycle and focus restoration ship here; room retention does not.
6. **0.5.0: Notes as the first bundled plugin and proof.** Depends on all preceding platform primitives. Expose existing document editing, scoped list/search, file/attachment transactions, paste/drop references and resource-specific trash/restore receipts, then extract capture, picker and editor workflow. **Hard prerequisite pulled forward from later undo work:** local text undo and an admitted Notes Restore action are required; shared undo contributions, history caps and restart persistence remain with 1201. Semantic color roles and contrast validation are also required by safe UI now; theme-package import/preview is later. Delete `PlexiApp` quick-note/picker workflow state, native overlays and direct host-only note actions after the public route passes the Notes proof. Publish readable source and qualify the same package as an independent install. Validate pack membership, help, command coverage and the installed 0.5.0 surface at its combined head.

Notes works offline and needs neither public secret-reference flows nor AI routing. Existing host credential storage, sealing/audit keys and channel recovery remain core responsibilities required by broker admission; this does not pull the new plugin secret API into 0.5.0. Existing AI/secret integrations retained in the product still use the one broker, but their new public plugin contributions are unavailable until their later slices qualify. Do not expose a legacy AI tier or raw-secret shortcut as a temporary v1 plugin API. No other later item is a hard prerequisite of the Notes proof.

### Releases 0.5.x or later: incremental expansion

1. **0.5.x or later: secret-reference flows.** Depends on the 0.5.0 broker, explicit bindings, operation lifecycle and secure host inputs. Publish opaque references, binding-specific values, rotation and admitted CLI environment mapping with recipient disclosure and negative tests. Preserve 0788/1179 ownership and the folder-secret gate. This precedes AI because the endpoint contract consumes a secret name through the host.
2. **0.5.x or later: fast/slow AI and OpenAI-compatible endpoints.** Depends on secret-reference use plus the broker, bounded network work and operation status. Delete legacy request tiers/conversion shims, the OpenRouter-only dispatch switch and dead provider-specific paths. Keep OpenRouter as a preset and keys, spend caps and ledger in the host. Qualify streaming, tools, cancellation, missing/unknown usage and concurrent reservations before advertising endpoints.
3. **0.5.x or later: undo contributions with 1201.** Depends on the broker, command registry, operation receipts and Notes restore primitive, not on AI. Add validated inverse/redo dispatch and shared history; 1201 owns its cap and restart persistence. Extend PluginHarness with history assertions. This precedes back burner so its reversible move can contribute undo without inventing a second history engine.
4. **0.5.x or later: 1183 back burner room.** Depends on commands, runtime supervision, status/attention and undo contributions, plus new room storage/events, cleanup leases and conditional close. Implement the settled per-context rooms, conditional zero-exit close and split recall proof. Delete `hide_pane`, hidden placeholders, hide/reveal/unhide and park/unpark commands, bindings and config aliases. A one-time reader may recover saved panes into explicit rooms without retaining the hidden-pane execution model. Extend the public harness for room/cleanup races and preserve live resources throughout migration.
5. **0.5.x or later: themes as data packages.** Depends on the package contract and 0.5.0 semantic color/contrast resolver; it does not depend on back burner or AI. Add no-code package validation, resource loading, preview/apply rollback and theme-change events. Keep agent synchronization a separately admitted optional code plugin. Default theme selection belongs to the V1 feature sweep.
6. **0.5.x or later: Focus Writing.** Depends on Notes document services, safe UI and placement; add scoped presentation leases and timer/settings support. Shared workflow undo uses 1201 where offered; text undo already belongs to the editor. Neither theme packages nor AI are prerequisites. Prove save conflicts, crash restoration and independent installation without a host workflow special case.
7. **0.5.x or later: remaining built-in workflow extraction.** Depends on each workflow's qualified public primitives and the V1 feature sweep's selection of retained experiments. Refine 1140 with scoped reveal and the bundled Needs you/Permissions UI, verified paginated permission history and supported provider-adapter delivery; none can manufacture grant authority. Secret settings depends on secret references; Agents depends on run projections; file browser/viewers depend on binding and bounded file/media services. Extract the remaining inventory workflows from `builtin_factory`, publish source and qualify independent installs/forks. Delete each native factory arm, private downcast and workflow call path at its extraction. Remove unretained experiments rather than granting exceptions. Keep disabled experiments disabled; the sweep decides optional survivors and the default theme. Validate pack/help/command coverage and installed acceptance for each release and the final combined V1 feature sweep.
8. **0.5.x or later: operation history export.** Depends on durable redacted broker outcome records from 0.5.0 and verified scoped history queries from the Permissions/history work. Add admitted export and richer search with pagination, retention semantics and redaction tests. Export is inert evidence, never a grant backup or replay endpoint. It is not a prerequisite of trusted consent, operation status or Notes recovery.

### Public rename and deletion boundaries

Package/view terminology changes in 0.5.0. AI tier removal belongs to the later AI slice; native workflow deletion belongs to each feature’s extraction. The release sequence owns those dependencies.

| Retired surface | Replacement and boundary |
|---|---|
| `app` package/lifecycle/state/tools commands | One `plugin` namespace, using the command split in Public entry points. Remove old commands, help entries, completion aliases and `app trust`; preapproval does not become valid by renaming it. |
| `app open/render/action`, including visualizer switches | `view open/render` for declarations and `pane action` for live pane targets; retained visualizers expose named views. No app-namespace aliases. |
| `[app]`, `[app.capabilities]`, overloaded manifest type | `[plugin]`, `[plugin.capabilities]`, explicit `[runtime]` and `[[contributes.*]]`; `PluginManifest` replaces the package model. Reject old schema shapes without a fallback parser. |
| Pack `[[app]]`, installed config discovery `apps/`, workspace `apps.toml` | `[[plugin]]`, `plugins/`, `plugins.toml`. Keep the pack definition as the membership source. This renames configuration discovery, not document addresses. |
| Public SDK/protocol app identities | Apply the named SDK renames above; audit wire variants, tool prefixes, event publisher fields, actor vocabulary and grant bindings by meaning. Expose plugin/instance/view/container IDs separately. Bump PGAP/WIT schemas and remove old import re-exports, enum spellings and conversion shims. Never widen a grant through string substitution. |
| App installation/authoring/catalog labels and routes | Plugins, Bundled, Marketplace; `/plugins` and `/docs/plugins` replace `/apps` and `/docs/apps`. Update navigation, templates, authoring guides, generated references, CLI skill examples, catalog/product fields and checkout/webhook contracts together. Preserve marketplace experiment gates; no live marketplace is claimed here. |
| Native package/UI owner names and extracted workflow paths | Rename package owners to Plugin and view owners to View; retain host `PlexiApp`, generic rendering and terminal ownership. Delete extracted factory arms and private shortcuts after public replacements qualify. |
| Low/Medium/High AI request tiers and provider switch | Fast/Slow plus the common endpoint type. Preserve the separate reasoning-effort setting and host ledger history. |

Public rename owners include `src/cli/args.rs`, help snapshots/completions and `v1_surface.txt`, `skills/plexi-cli/SKILL.md`, `sdk/python/plexi_sdk`, `sdk/wasm`, `src/protocol`, `wit/plexi.wit`, package/pack code, plugin manifests and website docs/catalog contracts. Regenerate derived references from their owners. Keep `manifest.toml`, `PACKAGE.toml`, `.plexipkg` and the `plexi_sdk` import name. In particular, keep existing `app_states` and notes document addresses under the host user-data contract; vocabulary changes do not rename user files.

A clean API break does not authorize deleting user documents. Legacy executable paths and authority fallback code go away; old portable data is preserved or moved aside with explicit, audited recovery where required. No ongoing dual writer or compatibility dispatcher survives.

## Appendix: draft implementation stint scopes

These are draft task scopes, not created tasks or progress records. Ian's rulings below are settled; task filing is a separate lead handoff. When filed through the create-stint workflow, each carries `v1`, links to 1182 and this document, names its release/dependencies and applies TESTING.md. Existing owning stints receive scope refinements rather than duplicate tasks. Each behavior-changing slice includes an info trace with IDs/outcomes and no secret payloads. Scope order follows the release sequence; independent scopes need not wait for unrelated earlier rows.

### 0.5.0 draft scopes

| Draft scope / existing owner | Dependency | Done-When and qualification |
|---|---|---|
| Public package/contribution contract and naming cutover | Settled naming and release boundary | Plugin/view CLI, manifests, SDK and wire identities agree on v1 with schema revisions and feature requirements. The surface remains unfrozen until the outside-author milestone. Old app commands/schema/import aliases are rejected; documents retain their addresses. Round-trip, help coverage and malformed fixtures cover the rename; later features report unavailable. |
| One admission authority | Contract identities | Production `PermissionStore` references, manifest grants and dual prompts are gone, including retained native workflows. Unverifiable grants are invalidated/re-requested; documents and audited secret recovery survive. Fixtures cover each effect family, forged callers, changed package/resource, revocation, audit failure, once replay and delegation narrowing. Installed bundled/external parity and trusted-consent controls pass. |
| Explicit folder bindings and targeting repairs | Contract and broker | Tests first reproduce invalid-target fallback, inactive-context launch and stale root reads. ID/path/local/global launch share one binding. Open in folder, retarget, restore and state-address parity pass A/B-folder negative controls. Installed board walkthrough proves the actual folder with remembered grants. |
| Supervised runtime and host work budgets | Contract and admitted handles | Python helper lifecycle, queue limits, fair servicing and cancellation meet the isolation budget under loop/flood/close fixtures. Installed terminal latency and occlusion evidence pass; inline-call/join controls fail. Native WASM remains experimental; remote/container runners remain deferred. |
| Commands and plugin contributions, refine 1151 | Contract and bindings | Registry exposes public contributions, explicit targets and unavailable reasons; one handler serves palette/key/CLI routes. Forged approval and stale target attempts fail. Installed keybinding walk and generated help agree. |
| Minimum trusted attention and operation lifecycle, refine 1140 | Broker and contract | Host consent/recovery, waiting UI, status query/events, atomic cancellation and durable redacted audit work without an inbox plugin. Stale click, pane/instance death, failed audit and fake provider-resolution controls fail. Installed wait/approve/deny/cancel walk passes. Broader reveal, history browsing and plugin extraction remain later. |
| Public safe UI, refine 1207 | Contract; native kit API settled | Host, bundled and independent fixtures use the same allocator/color resolver. Geometry, accessibility, action routing, limits and contrast pass semantic transport fixtures; narrow/scale/reduced-motion scenes are inspected. No private structs from another branch. |
| View placement and public PluginHarness | Contract, commands, safe UI, runtime | Every declared view renders in every slot/size class on host harnesses. Override/remember/focus rules and Cmd+O picker/editor example pass. Events, fake broker decisions, attention, local undo/restore and toast assertions retain screenshot metadata; stale placement/blank-action controls fail. Test injection is unavailable in production. |
| Document services and first bundled Notes plugin | Broker, bindings, runtime, commands, placement and safe UI/test kit | Notes proof passes through public calls with existing notes preserved and native workflow state deleted. Transactions, drops, clipboard, file conflicts, text undo and receipt-based Restore have negative controls and installed proof. Source and independent install demonstrate the same authority; shared history and public AI/secrets are not prerequisites. |

### 0.5.x or later draft scopes, in recommended order

| Draft scope / existing owner | Dependency | Done-When and qualification |
|---|---|---|
| Secrets by reference, integrate 0788/1179 | 0.5.0 broker, bindings, operation lifecycle and secure inputs | API key use, per-binding values, explicit CLI environment mapping and rotation pass denial/stale-reference tests. Plaintext stays outside plugins and agent environments; native-recipient authority is disclosed. Folder gates and channel ownership hold. Throwaway-keychain and signed installed checks remain distinct from PR file-backed keys. |
| Fast/slow AI and OpenAI-compatible endpoint routing | Secret references, broker and bounded network operations | SDK/PGAP/WIT accept fast/slow only; Medium and old conversions are deleted. OpenRouter and another protocol fixture share the base-URL/secret-name client. Routing, streaming/tools, cancellation, missing keys, changed endpoints, unknown cost and concurrent caps pass. Ledger attribution remains host-owned; dead local/Ollama paths are removed and references regenerated. |
| Undo contribution contract, refine 1201 | 0.5.0 broker, commands, operations and Notes restore receipts | Apply/undo/redo round-trip and public harness history assertions pass. Stale preconditions, missing contributors, revoked grants and unknown outcomes do not move history incorrectly. Text Cmd+Z wins; acknowledged inverses produce named toasts. History cap and restart persistence are decided and qualified within 1201. |
| Back burner, refine 1183 | Runtime, commands, attention/status and 1201; add rooms/events and cleanup leases | Reproduce the existing failure, then prove per-context rooms, conditional zero-exit close, nonzero attention, split recall, late completion, process identity and restart behavior. Extend harness room assertions. Help/source coverage proves old hide/park paths removed; installed Ian acceptance remains explicit. |
| Data theme packages | 0.5.0 package contract and color roles | Load validation, preview rollback and no-code/no-capability rejection pass. Bundled/external data packages behave identically. Agent synchronization stays a separate code plugin; default-theme selection stays with the sweep. |
| Focus Writing example | Notes document services and safe UI/placement; 1201 for shared workflow undo | Add presentation leases and timers/settings; prove crash release, external edit conflict, local undo and offline use without a host special case. An independent install exercises the same APIs and terminal responsiveness. |
| Remaining core-plugin extraction and public-only audit, including 1140 | Qualified primitives per workflow and selected feature sweep | Scoped reveal, provider delivery receipts and verified paginated history pass missed-event/restart and forged-action controls. Needs you, Permissions, Agents, secrets/settings, browser, viewers, gallery and retained renderers pass domain-specific installed walks. Pack packages have no private imports, bundled bypass or native workflow factory arms. Sweep selects optional survivors/default theme; disabled experiments stay disabled. |
| Operation history export and richer search | Durable 0.5.0 audit plus verified history query from Permissions work | Scoped pagination, redaction, retention and export round-trips pass. Exported evidence cannot mint, restore or replay authority. Failed/partial/unknown outcomes retain their meaning; installed export uses no raw grant-store access. |

## Settled decisions and ownership

Decided by Ian on 2026-11-10, per `astra-1182-revision-1.md`: option A plugin/view naming; Plugins/Bundled/Marketplace labels; no app-namespace aliases; agents as separate principals; Python on Wasmtime inside supervised helpers for V1; native WASM experimental and native dynamic libraries excluded; portable helper IPC with remote/container runners deferred; fast/slow AI and one OpenAI-compatible endpoint type with OpenRouter preset; host-owned keys, caps and ledger; the name back burner; placement at view opening with host slots and size classes; a public host-based developer test kit; and concrete secret-reference use cases with channel ownership.

Ian's revision 2 ruling settles invalidation and re-request of legacy grants that cannot prove exact authority, preserving documents and audited secret recovery; per-context back burner rooms with conditional zero-exit close and split recall; and the release sequence above. Ian is the primary plugin developer, 0.5.0 is the minimum real-plugin surface with Notes, and v1 is not frozen until an outside author ships a plugin.

Undo cap and restart persistence stay with 1201. The V1 feature sweep owns the default theme and optional experiment selection. Those are delegated decisions within their own scopes, not unresolved choices for this design. There are no open questions for Ian in 1182.

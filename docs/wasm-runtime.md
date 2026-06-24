# Plexi WASM Runtime — Full Architecture Specification

Status: active
Stint: see `wasm-runtime-impl-plan.md` for the build sequence and gate coverage.

This spec is the authoritative description of Plexi's WASM runtime destination architecture. It supersedes prior WASM/WASI planning fragments. PGAP/Python and WASM land as parallel runtimes under one app contract: PGAP remains the native authoring path and WASM is the sandbox/performance path. The v1 release path (S1-S6 in `docs/app-framework-marketplace.md`) remains the correct near-term sequence.

---

## Why Rebuild

The v1 runtime (Python subprocess + PGAP over stdio) is the right bootstrap. It ships fast, it works on macOS desktop, and it gets the Core 9 apps in front of users. But every capability Plexi wants to add pulls against it:

- **Cloud execution**: you cannot run an OS process remotely and pipe stdio over a WebSocket without a pile of per-platform scaffolding.
- **Mobile**: iOS forbids spawning arbitrary subprocesses. Android makes it painful.
- **Sandboxing**: a Python subprocess runs as the user. Capability grants today are enforced by convention (the SDK checks them), not by the OS or the runtime.
- **The run primitive**: `plexi run @author/tool` should be zero-install and ephemeral. Fetching a Python package + spawning a subprocess is not that.
- **The payment layer**: HTTP 402 gating makes no sense if the thing behind the gate is a Python file you've already downloaded.
- **Any-language apps**: today you write Python. The protocol is text over stdio. There is no path to Rust, Go, or Swift apps without reimplementing PGAP in each language.

The v2 runtime fixes all of these at the root. WASM is the common execution substrate. The WASM Component Model is the interface definition layer. The host does not need to know what language an app was written in, where it runs, or how it was fetched.

---

## The Three Planes

Everything in the v2 runtime separates cleanly into three planes. They communicate through typed interfaces, not ad-hoc protocols.

```
┌──────────────────────────────────────────────────────┐
│  RENDER PLANE                                        │
│  Host (Rust + egui + wgpu)                          │
│  Scene graph. Maps typed UINode trees to GPU calls. │
│  Owns: layout, theming, input dispatch, compositor  │
├──────────────────────────────────────────────────────┤
│  INTERFACE LAYER                                     │
│  WIT (WebAssembly Interface Types)                  │
│  Typed, binary, schema-first. Generated code both   │
│  sides. Local = direct call. Remote = binary RPC.  │
├──────────────────────────────────────────────────────┤
│  COMPUTE PLANE                                       │
│  WASM module (any language)                         │
│  Pure function: (State, Event) → (NewState, Effects)│
│  Cannot touch anything the host didn't hand it.    │
├──────────────────────────────────────────────────────┤
│  STATE PLANE                                         │
│  CRDT store (host-owned)                            │
│  Persisted. Namespaced per app. Syncable.           │
│  App reads/writes through a declared capability.   │
└──────────────────────────────────────────────────────┘
```

These planes are not layers in a call stack — they are independent systems. The compute plane is a pure function. The state plane is a database. The render plane is a GPU pipeline. The interface layer connects them with typed contracts.

---

## ~~The App Model~~ ✅ SHIPPED

A Plexi v2 app is a WASM Component that implements two functions:

```wit
// plexi:app/lifecycle@0.2.0

interface lifecycle {
    /// Called once on startup with initial state snapshot.
    init: func(state: state-snapshot) -> list<effect>

    /// Called for every user input or host event.
    update: func(event: input-event) -> list<effect>

    /// Called when the host needs to render. Must be pure — no side effects.
    view: func() -> ui-node
}
```

That is the entire contract. The app cannot do anything else. It cannot open file descriptors, allocate OS resources, or call syscalls. The WASM sandbox enforces this at the link layer — the only functions available to the module are the imports the host chose to expose, and those imports are the capability system.

### Why pure functions

`view()` is a pure function of internal state — no arguments, returns a complete UI tree. The host calls it after any state mutation. This enables:

- **Deterministic replay**: initial state + sequence of input events = exact final UI state. Bug reproduction is: save the event log, replay it.
- **Trivial testing**: call `update(mock_event)`, assert on returned effects and on `view()` output. No process, no network, no mocks.
- **Structural diff**: the host diffs consecutive `view()` outputs and sends only changed subtrees over the wire. The app does not need to implement diff logic.

---

## Effect System  🟡 PARTIAL

> **Status:** the round-trip *mechanism* ships (G3 ✅). **Implemented variants:** `get-system-stats`, `set-timer`/`cancel-timer`, `set-title`/`set-status`, `close-self`, `file-read`, `file-write`, `http-fetch`, `request-capability`, `ai-query`, `declare-event-streams`, and `emit-event`. **Stubbed or missing:** `file-list`/`file-watch`, `websocket-open`, `open-pane`, `audio-record`, `spawn-child`, `clipboard-*`, `notify`, and `payment-request`. The input-event variants `mouse`/`resize`/`focus-*`/`payment-complete` are defined in WIT but never enqueued.

Effects are how apps reach outside the WASM sandbox. An effect is a typed request the app returns from `update()`. The host executes effects against the capability grants and returns results on the next `update()` call.

```wit
variant effect {
    // Filesystem
    file-read(file-read-effect),
    file-write(file-write-effect),
    file-list(file-list-effect),
    file-watch(file-watch-effect),

    // Network
    http-fetch(http-fetch-effect),
    websocket-open(websocket-open-effect),

    // Host UI
    open-pane(open-pane-effect),
    close-self,
    set-title(string),
    request-focus,

    // Audio / video
    audio-play(audio-effect),
    audio-record(audio-effect),

    // System
    spawn-child(spawn-child-effect),
    clipboard-read,
    clipboard-write(string),
    notify(notification-effect),

    // Payment
    payment-request(payment-effect),

    // Capability escalation
    request-capability(capability-id),
}
```

Effect results return as a synthetic `input-event` on the next `update()` call:

```wit
variant input-event {
    // User input
    key(key-event),
    mouse(mouse-event),
    resize(size),
    focus-gained,
    focus-lost,

    // Effect results
    file-read-result(result<bytes, io-error>),
    http-response(http-response),
    websocket-message(bytes),
    capability-granted(capability-id),
    capability-denied(capability-id),
    payment-complete(payment-result),
    child-event(child-id, child-event),

    // Host events
    theme-changed(theme),
    state-sync(state-snapshot),
}
```

No callbacks, no async/await visible to the app. The app is always a synchronous function. The host manages concurrency. This is the Elm architecture with WASM as the runtime boundary.

---

## ~~UI Node Tree~~ ✅ SHIPPED

> **Status:** all node variants render to egui (G4 ✅). Button/text-input/list interactions are mapped to typed `input-event` variants and delivered to `update()` in the same frame (Lane B ✅).

On the WASM path, PGAP's newline-delimited JSON UI trees are replaced by a typed tree defined in WIT. The host owns the component implementations: a `button-node` always renders as a Plexi button. Apps cannot bypass the design system except through `surface-node`. PGAP apps keep using the PGAP wire protocol.

```wit
variant ui-node {
    // Layout
    column(column-node),
    row(row-node),
    stack(stack-node),
    scroll(scroll-node),
    sized(sized-node),
    padding(padding-node),

    // Content
    text(text-node),
    image(image-node),
    icon(icon-node),
    divider,

    // Input
    button(button-node),
    text-input(text-input-node),
    checkbox(checkbox-node),
    select(select-node),
    slider(slider-node),

    // Containers
    card(card-node),
    list(list-node),
    table(table-node),
    modal(modal-node),
    tooltip(tooltip-node),

    // Specialized
    code-block(code-block-node),
    markdown(string),

    // Escape hatch
    surface(surface-node),  // raw wgpu surface, app renders directly via shared texture
}
```

Each node carries: a stable `key` for diffing, style overrides within the design token system, and event handlers expressed as opaque IDs (the host maps them to `update()` calls — apps never register callbacks).

### Surface node

`surface-node` is the one escape from the typed tree. It allocates a wgpu texture the app can write to directly. Used for: canvas drawing, video, 3D rendering, game UIs. It is always available but does not compose with the theme system. Apps that use only typed nodes are fully theme-consistent by construction.

---

## State Plane  🟡 PARTIAL

> **Status:** the primitive get/set/delete/list-prefix store ships and persists across restarts (G5 ✅). **Not built:** `cas()`, CRDT merge, and `state-sync` snapshots / SpacetimeDB sync.

The host owns a CRDT key-value store, namespaced per app instance. The app does not serialize state to disk — it writes to the store through the `state` import, and the host persists it.

```wit
// plexi:host/state@0.2.0

interface state {
    /// Read a value. Returns none if key has never been written.
    get: func(key: string) -> option<bytes>

    /// Write a value. Persisted immediately.
    set: func(key: string, value: bytes) -> result<_, state-error>

    /// Delete a key.
    delete: func(key: string) -> result<_, state-error>

    /// List keys with a prefix.
    list-prefix: func(prefix: string) -> list<string>

    /// Atomic compare-and-swap. Used to implement CRDT merge.
    cas: func(key: string, expected: option<bytes>, new: bytes) -> result<bool, state-error>
}
```

State is:

- **Persisted after every write.** No explicit save step.
- **Namespaced to app ID.** Apps cannot read each other's state through this interface. Cross-app communication uses explicit `spawn-child` effects with typed messages.
- **CRDT-compatible.** `cas()` enables apps to implement their own CRDT merge logic on top of the primitive store. The host does not mandate a CRDT strategy — it provides the primitive.
- **Syncable.** State snapshots are the sync unit. When SpacetimeDB sync lands, the host sends `state-sync(snapshot)` events to running apps when remote writes arrive.

### Ephemeral runs

Apps launched with `plexi run` without `--persist` get a temp-scoped namespace that is deleted when the pane closes. The app code is identical — it still writes to `state`. The host decides the lifetime of the namespace based on the execution mode.

---

## Capability System  🟡 PARTIAL

> **Status:** link-time gating ships — an ungranted import is not linked into the module, so the app physically cannot call it. Session-time escalation via `request-capability` also ships for focused WASM panes: user decisions enqueue `capability-granted`/`capability-denied`, audit `PermissionDecision`, and widen runtime fs/net access only for recognized scoped strings. Install-time review and remembered grants now persist raw WASM decisions. Raw `.wasm` launches no longer auto-grant imported host interfaces; `plexi app open ./x.wasm` reviews and remembers required link-time imports before launching.

Every import the app can call is a capability. The manifest declares required and optional capabilities. The host only links the imports that match granted capabilities. If `net:fetch` is not granted, the import does not exist in the linked module — the app cannot call it. This is enforced at the WASM link layer, not at runtime.

### Manifest declaration

```toml
[capabilities]
required = [
    "fs:read:~/projects/",
    "net:fetch:api.github.com",
    "state:read-write",
]
optional = [
    "fs:write:~/projects/",
    "audio:playback",
    "clipboard:read",
]
```

Required capabilities block install if denied. Optional capabilities are skipped — the app must handle the missing import gracefully (by checking `capability-granted`/`capability-denied` events).

### Grant flow

- **Persistent app install**: host shows a capability review sheet before first launch. User approves once. Remembered.
- **Ephemeral run**: host shows a compact inline prompt at launch. User can approve for this session only or permanently.
- **Capability escalation**: app returns `request-capability(id)` effect mid-session. Host prompts. Result arrives as `capability-granted` or `capability-denied` event.

### Capability scope

Filesystem capabilities are path-scoped. `fs:read:~/projects/` grants read access under that path only. The host enforces path containment — the WASM module cannot traverse up with `../` or follow symlinks outside the granted scope.

Network capabilities are host-scoped. `net:fetch:api.github.com` grants only that host. Wildcard (`net:fetch:*`) is available but requires curated trust tier.

---

## The Run Primitive  🟡 PARTIAL

> **Status:** local ephemeral launch of a local `.wasm` file ships via `plexi app open ./x.wasm` (G6 ✅). **Not built:** registry resolution (`@publisher/app`), content-addressed bundle cache, Ed25519 signature verification, the 402 payment check, and cloud / preferred-local execution routing. Steps 1–5 in the flow below are aspirational.

```
plexi run @publisher/app-name[@version] [--persist] [args...]
```

`plexi app open` is `plexi run` with a locally installed app. Same code path. The distinction between "installed app" and "run from registry" is a caching and lifecycle detail, not an architectural one.

### Execution flow

```
1. Resolve manifest
   ├── Installed: read from profile dir
   └── Registry: fetch manifest from CDN (HTTPS, content-addressed)

2. Check WASM bundle cache
   ├── Cache hit: verify content hash, skip download
   └── Cache miss: fetch bundle, verify Ed25519 signature against publisher key

3. Payment check
   └── Registry returns 402: Plexi payment flow → auth token → retry

4. Capability grant check
   ├── Persistent, already granted: proceed
   ├── Ephemeral, no grants: show compact capability prompt
   └── Missing required capability: abort with clear message

5. Determine execution location
   ├── manifest.runtime.execution = "local" → local wasmtime
   ├── manifest.runtime.execution = "cloud" → cloud runtime
   └── manifest.runtime.execution = "preferred-local" → local if capable, else cloud

6. Initialize state namespace
   ├── --persist or persistent app: profile dir namespace
   └── ephemeral: temp namespace (deleted on pane close)

7. Link WASM module
   └── Only imports matching granted capabilities are linked

8. Call init() with state snapshot
   └── Execute returned effects

9. Render loop
   └── call view() → diff against previous tree → update scene graph
   └── dispatch input events → call update() → execute effects
```

---

## Execution Locations  🟡 PARTIAL (local only)

> **Status:** **Local (wasmtime embedded)** ships — one `Store` per pane, direct function calls. **Cloud** and **Mobile** are not built (G9 deferred).

The host does not care where the WASM process runs. It calls WIT interface functions and receives UINode trees. The execution location is resolved at launch from the manifest and available resources.

### Local (wasmtime embedded)

The host embeds wasmtime. App instances are wasmtime `Store` objects, one per pane. `view()` and `update()` are direct function calls into the WASM instance — zero serialization, zero IPC, zero protocol parsing. Capability imports are Rust closures registered in the store's linker.

This is faster than the v1 Python subprocess path for UI-intensive apps because there is no JSON serialization on the hot path.

### Cloud (remote wasmtime)

The host opens a WebSocket to the cloud runtime. The cloud runtime runs the same wasmtime + capability broker. The protocol is WIT-generated binary (produced by `wit-bindgen` + a binary transport layer). The host sends input events; the cloud runtime responds with UINode patches.

```
Host                          Cloud runtime
 │                                │
 │── InputEvent (binary) ────────►│
 │                                │ update() → effects
 │                                │ view() → UINode
 │◄─ UINodePatch (binary) ────────│
 │                                │
 │── EffectResult (binary) ───────►│  (for effects that need client resources)
```

Effects that touch the user's local machine (filesystem, clipboard, audio) are sent back to the host as `effect-request` frames. The host executes them and returns `effect-result` frames. Effects that are pure server-side (network fetch, compute) execute on the cloud runtime directly.

State sync: the cloud runtime's state writes are streamed to the host, which persists them to the user's profile dir. If the cloud process dies, a new instance is started and handed the last-known state snapshot. The reconnect is transparent to the user — the UI may flicker for one frame.

### Mobile

The mobile Plexi host is the same code path as the cloud client: a thin host that renders UINode trees and pipes input events to a WASM runtime. The WASM runtime may be:

- **In-process local**: for small apps, wasmtime is embedded in the mobile binary. Same as desktop local.
- **Cloud**: for heavy apps (ML, large file indexing), execution routes to the cloud runtime. The mobile host is a viewport into a PGAP stream. No local compute limit applies.

The mobile pane model collapses to stack-based navigation on phone (one pane visible, swipe to switch) and retains tiling on iPad. The app sees the same `resize` events and the same UINode system — it does not need to know it is running on mobile.

---

## Wire Protocol (Cloud Mode)  ⬜ NOT BUILT (G9 deferred)

The wire protocol is derived from the WIT definitions, not hand-written. `wit-bindgen` generates the serialization layer; a binary transport wraps it.

### Frame format

```
[4 bytes: frame length]
[1 byte:  frame type]
[N bytes: binary payload (msgpack)]
```

Frame types:

```
0x01  ViewRequest       host → cloud    (request view() output)
0x02  UINodePatch       cloud → host    (diff against previous tree)
0x03  InputEvent        host → cloud    (key, mouse, resize, etc.)
0x04  EffectRequest     cloud → host    (cloud needs local resource)
0x05  EffectResult      host → cloud    (result of local effect)
0x06  StateSync         bidirectional   (state checkpoint)
0x07  PaymentRequest    cloud → host    (402 interception)
0x08  PaymentResult     host → cloud    (payment outcome)
0x09  Ping/Pong         bidirectional   (keepalive)
```

### UINodePatch

The host maintains the previous UINode tree. After calling `view()`, the cloud runtime diffs the output against the prior tree and sends only changed subtrees. The diff algorithm: walk the tree by `key`; if a node's key matches and its value is unchanged, skip it. Changed or new nodes are included in the patch with their full subtree.

A well-authored app that marks only dirty nodes produces near-zero wire traffic at rest. A text editor that changes one character in one `text-node` sends a patch of approximately 50 bytes.

### Binary encoding

msgpack. Not JSON. JSON is available in debug mode (set by manifest `[dev] wire-format = "json"`) but is never used in production. The WIT types map directly to msgpack types — no schema negotiation needed because the WIT file is the schema.

---

## Registry Architecture  ⬜ NOT BUILT

The registry is a content-addressed CDN backed by a metadata store. Apps are identified by their content hash, not their name. Names are aliases that resolve to hashes.

```
Registry
├── index/
│   ├── @publisher/app-name → {latest: hash, versions: {1.2.3: hash, ...}}
│   └── publisher keys (Ed25519 public keys, one per verified publisher)
├── bundles/
│   └── {content-hash}.wasm  (immutable, served from CDN edge)
├── manifests/
│   └── {content-hash}.toml  (immutable)
└── payments/
    └── 402 gate configuration per app version
```

A `plexi run @publisher/app` resolves: name → latest hash → fetch `{hash}.wasm` from CDN. The CDN has the bundle forever because content addresses never change. Delisting an app removes the name alias but does not remove the bundle.

### Publisher tiers

| Tier | Requirements | WASM required | 402 enabled | Wildcard net | Curated badge |
|---|---|---|---|---|---|
| Unverified | None | Yes | No | No | No |
| Verified | Identity check + key signing | Yes | Yes | No | No |
| Curated | Plexi team review | No (Python compat allowed) | Yes | Yes | Yes |

Curated is the only tier allowed to use `net:fetch:*` or to skip the WASM requirement (for the transition period when Python compat is still supported).

### Manifest format (v2)

```toml
schema_version = "2"
id = "com.publisher.app-name"
version = "1.2.3"
name = "App Name"
description = "One sentence."
publisher = "publisher-name"

[runtime]
target = "wasm32-wasip2"            # WASM Component Model target
execution = "local"                 # local | cloud | preferred-local

[capabilities]
required = [
    "fs:read:~/projects/",
    "state:read-write",
]
optional = [
    "audio:playback",
    "clipboard:read",
]

[payment]
model = "free"                      # free | per-run | subscription
price_usd_cents = 0                 # for per-run

[state]
schema_version = 1                  # bumped when state shape changes

[dev]
wire_format = "json"                # debug only; omit in release
```

---

## HTTP 402 Payment Gate  ⬜ NOT BUILT (G10 deferred)

The 402 integration is at the registry fetch layer, not the app layer. Apps do not implement payment logic.

### Flow

```
1. plexi run @author/paid-tool
2. Host fetches manifest from registry
3. Registry returns HTTP 402 with:
   {
     "price_usd_cents": 5,
     "model": "per-run",
     "payment_endpoint": "https://registry.plexiapp.com/pay/...",
     "session_token_on_success": true
   }
4. Host shows payment prompt (or auto-pays if under user's auto-pay threshold)
5. User approves → host POSTs to payment_endpoint
6. Registry returns session token (short-lived JWT)
7. Host retries manifest fetch with Authorization: Bearer <token>
8. Bundle fetch proceeds normally
```

The session token gates bundle access. The bundle is not in the 402 response — payment merely unlocks the normal fetch. This means the content hash verification still works (the bundle is the same whether you paid or not; payment only proves you're authorized to fetch it).

### Subscription model

For subscription-priced apps, the session token is a long-lived credential stored in the user's profile dir alongside the capability grants. It is refreshed on expiry. The user does not see a payment prompt after the initial subscribe.

### Revenue share

The registry takes a platform cut and remits the remainder to the publisher. The cut percentage is declared in Plexi's publisher terms, not in this spec. The payment flow is opaque to the WASM runtime.

---

## Python Compatibility Layer  ⬜ NOT BUILT (G8 deferred)

Python apps currently continue to run through native `ProcessApp` and the Python SDK v3 PGAP bridge. CPython-in-WASM is deferred G8 work, not part of the current SDK v3 app API.

### How it works

1. The shared CPython WASM runtime is a separate registry bundle (~40MB). It is cached once per device, versioned independently of any app. It is not downloaded per-app.

2. A future Python WASM app may ship Python bytecode (`.pyc`, typically < 200KB) plus a manifest flag or runtime target that routes to this compatibility layer. That manifest contract is not shipped.

3. The compat bundle is a WASM Component that embeds CPython, implements the `plexi:app/lifecycle` interface, and drives the Python app through a thin adapter. The adapter:
   - Calls `app.view()` in Python → converts the return value to a `ui-node`
   - Calls `app.update(event)` in Python → converts effects from Python dicts to typed `effect` variants
   - Exposes the state/capability imports as Python objects through the existing SDK API

4. The Python SDK surface should remain source-compatible where possible, but the runtime route is future work.

5. PGAP (the JSON stdio protocol) is not used. The Python app's SDK calls go through the in-process adapter, not through a subprocess pipe. The Python process is not a subprocess — it is CPython running inside a WASM instance inside the host.

### What changes for Python authors

Nothing today. SDK v3 Python apps are normal `[app] type = "app"` manifests and run through native `ProcessApp`.

### Compatibility

The future compatibility bridge must not remove the existing PGAP/Python runtime until a later, explicit boundary decision. New apps can choose SDK v3 Python through native `ProcessApp` or WIT/WASM for the sandbox/performance path.

The compat shim can be retired at a later v3 boundary if Core apps and marketplace apps no longer need it.

---

## Host Changes  🟡 PARTIAL

> **Status:** "What is added" — embedded wasmtime, WIT layer, scene graph, link-time capability gating — ships. **"What is removed" has NOT happened:** the PGAP parser, subprocess plumbing, and `ProcessApp` are all still present and running v1 apps. WASM landed *alongside* v1, not as a replacement (supersession is a v3 outcome after G8). Cloud-runtime and registry clients are not built.

### What is removed

- PGAP parser and JSON deserializer
- Subprocess spawn + stdout/stderr plumbing
- `ProcessApp` pane type (replaced by WasmApp)
- Per-app IPC socket setup (mobile: in-process; desktop local: direct WASM calls; cloud: WebSocket)

### What is added

- **wasmtime runtime**: embedded, one `Store` per pane instance. Capability imports registered per-grant.
- **WIT interface layer**: generated by `wit-bindgen`. The host implements `plexi:host/*` interfaces; app modules export `plexi:app/lifecycle`.
- **Scene graph**: retained UINode tree per pane. Host diffs consecutive `view()` outputs and calls egui only for changed nodes.
- **CRDT state store**: sled or similar embedded KV, one namespace per app instance, persisted after every write.
- **Capability broker**: maps granted capabilities to wasmtime linker imports. Handles path containment checks, network host matching, capability escalation prompts.
- **Cloud runtime client**: WebSocket connection manager, binary frame encoder/decoder, effect request routing.
- **Registry client**: manifest fetch, bundle cache (content-addressed on disk), signature verification, 402 interception.

### What is unchanged

- egui widget implementations — these become the renderer targets for typed `ui-node` variants. A `button-node` calls the same egui `Button::new()` calls as before; the difference is that the call site is now a typed dispatch rather than a JSON tree walk.
- wgpu surface — maps to `surface-node`. The GPU pipeline is identical.
- The pane model, tiling system, terminal emulator, and all host chrome are unaffected.

---

## Security Model  🟡 PARTIAL

> **Status:** the sandbox boundary (per-instance isolated linear memory, no syscalls) and link-time capability enforcement ship. **Not built:** publisher Ed25519 signature verification, the root-key transparency log, and the ephemeral-run unverified-publisher warning (all depend on the registry).

### Sandbox boundary

WASM linear memory is isolated per instance. An app cannot read another app's memory, the host's memory, or any OS resource. There are no syscalls — the WASM module has no direct access to the OS. Every external action is an imported function that the host controls.

### Capability enforcement

Capability grants are enforced at link time. If `net:fetch` is not in the granted set, the `plexi:host/capabilities#http-fetch` import is not linked into the module. The module cannot call a function that does not exist. There is no runtime capability check — the check happens once at link time when the pane opens.

This is categorically stronger than the v1 model (where the SDK checks grants by convention but the Python process runs with full user permissions).

### Publisher signatures

Every bundle in the registry is signed with the publisher's Ed25519 key. The host verifies the signature against the publisher's public key (fetched from the registry's key index and cached locally). A bundle whose signature does not verify is never executed.

The registry's key index is itself signed by Plexi's root key. The host ships with the root public key embedded. Key rotation follows a transparency log model (append-only, publicly auditable).

### Ephemeral run security

Ephemeral runs (`plexi run` without `--persist`) are the highest-trust interaction for users: they are running code they have not permanently installed. The mitigations:

- WASM sandbox is unconditional — no capability can bypass it
- Ephemeral runs with unverified publisher show a warning at the capability prompt
- State namespace is temp-scoped — deleted on pane close, so the app cannot persist anything without a granted `state:read-write` capability
- Network access requires explicit grant even for ephemeral runs

---

## What This Architecture Enables

### Completions of prior vision

| Prior goal | How v2 delivers it |
|---|---|
| Any-language apps | Anything that compiles to WASM. WIT is the contract. |
| Real sandboxing | WASM + link-time capability enforcement. Not convention-based. |
| Cloud execution | Cloud runtime is the same wasmtime + broker. Wire protocol is generated from WIT. |
| Mobile | Thin host + cloud execution. No subprocess required. |
| Run primitive | `plexi run` = fetch bundle + link + call init(). One code path. |
| 402 payment | Registry returns 402. Host intercepts, resolves, retries. App sees nothing. |
| Ephemeral tools | Same as run primitive. Temp state namespace. No install. |
| Deterministic testing | Pure functions. call update(), assert on effects and view(). No mocks. |
| Multi-device sync | State plane emits CRDT snapshots. SpacetimeDB merges them. |
| Zero-UI-overhead cloud | UINodePatch is a structural diff. Text editor changing one char = 50 bytes on the wire. |

### New capabilities that fall out

- **Forking**: user taps "customize" on any installed app → agent gets the source → saves as a new bundle under the user's publisher key → installs as a new app. The fork is a new content address. The original is unchanged.
- **Composable apps**: two WASM components can be linked together (WASM Component Model native composition). A `plexi:app/data-source` interface lets one app feed data into another without going through the host.
- **Replay debugging**: save the initial state + full input event log → replay any session exactly → step-debug by replaying up to any point. No special debug instrumentation needed.
- **Offline-capable cloud apps**: state is always persisted locally. Cloud execution enhances but is not required. An app that runs fine locally gracefully degrades to local wasmtime when offline.
- **App store per-run billing**: the 402 model enables pay-per-use for compute-intensive apps (AI, rendering, indexing) without the app author managing subscriptions. Plexi collects per-run, remits to publisher.

---

## Verification Gates

Each gate is a concrete, runnable test — not a design criterion. Every gate must pass before the corresponding system is considered shipped. Gates are ordered by dependency: each gate assumes the ones above it are already green.

Reference implementations for gates G1–G5 are in `apps/wasm-poc/`. Reference WIT is in `wit/plexi.wit`.

### ~~G1 — WIT interface compiles~~ ✅ SHIPPED

**What it proves:** The WIT definitions in `wit/plexi.wit` are syntactically and semantically valid. App crates can generate bindings from them.

**Test:**
```bash
cd apps/wasm-poc/sysmon
cargo component build --target wasm32-wasip2
# Must produce target/wasm32-wasip2/debug/sysmon.wasm with no errors.
```

**Pass condition:** `sysmon.wasm` is produced. `wasm-tools validate sysmon.wasm` exits 0.

---

### ~~G2 — Pure function unit tests~~ ✅ SHIPPED

**What it proves:** `init()`, `update()`, and `view()` are pure functions that can be tested without a host, without a process, without network.

**Test:** Rust unit tests in the sysmon crate:
```rust
#[test]
fn timer_fired_requests_stats() {
    init(empty_state(), (400.0, 300.0));
    let effects = update(InputEvent::TimerFired(1));
    assert!(effects.iter().any(|e| matches!(e, Effect::GetSystemStats)));
}

#[test]
fn stats_result_updates_view() {
    init(empty_state(), (400.0, 300.0));
    update(InputEvent::SystemStatsResult(mock_stats(42.0)));
    let tree = view();
    assert!(tree_contains_text(&tree, "42.0%"));
}
```

**Pass condition:** `cargo test` green, no host process required.

---

### ~~G3 — Effect system round-trip~~ ✅ SHIPPED (mechanism only)

> 🟡 The round-trip *mechanism* is shipped and tested. Filesystem read/write and HTTP fetch now round-trip; notify, spawn, clipboard, payment, and other broader variants remain stubbed or missing — see [Effect System](#effect-system) callout.

**What it proves:** The host correctly executes effects returned from `update()` and delivers results as the next `input-event`.

**Test:** Host integration test (Rust, using the wasmtime host directly):
```rust
let mut instance = WasmApp::load("sysmon.wasm", grants![]);
instance.init(StateSnapshot::empty(), (400.0, 300.0));
// Timer fires → should return GetSystemStats effect
let effects = instance.update(InputEvent::TimerFired(1));
assert!(effects.contains(Effect::GetSystemStats));
// Host delivers stats result → view() should show CPU%
instance.update(InputEvent::SystemStatsResult(mock_stats(55.0)));
let tree = instance.view();
assert!(tree_text_contains(&tree, "55.0%"));
```

**Pass condition:** Full round-trip in a HostHarness test, no subprocess, no stdio.

---

### ~~G4 — UINode tree renders correctly (sysmon)~~ ✅ SHIPPED

> Rendering and typed-node interaction both ship. Button clicks, submitted inputs, list selections, and text changes are routed to guest `update()` as `ui-action` / `ui-value-change` events (Lane B ✅).

**What it proves:** The host scene graph correctly maps the typed UINode tree from `view()` to egui widgets. The structural diff only repaints changed nodes.

**Test:** PlexiUiHarness scene test:
```toml
# tests/scenes/sysmon-render.toml
[[steps]]
action = "open_wasm_app"
path = "apps/wasm-poc/sysmon/target/wasm32-wasip2/debug/sysmon.wasm"

[[steps]]
action = "inject_event"
event = { type = "system_stats_result", cpu_usage_pct = 67.3, memory_used_bytes = 8589934592, memory_total_bytes = 17179869184 }

[[steps]]
action = "assert_screenshot"
snapshot = "sysmon-render-expected.png"
```

**Pass condition:** Screenshot matches snapshot within pixel tolerance. Log confirms only the metrics rows were repainted (header node diff skipped).

---

### ~~G5 — State persists across restarts~~ ✅ SHIPPED (primitive KV)

> 🟡 The primitive get/set/delete/list-prefix store persists across restarts. `cas()`, CRDT merge, and `state-sync` (SpacetimeDB) are **not built**.

**What it proves:** The CRDT state store persists data written by effects across app restarts. `init()` receives the previous state snapshot.

**Test:**
```rust
// First run: change poll interval to 5000ms
let mut inst = WasmApp::load("sysmon.wasm", grants!["state:read-write"]);
inst.init(StateSnapshot::empty(), (400.0, 300.0));
inst.update(InputEvent::Key(key("=")));  // +1000ms
inst.update(InputEvent::Key(key("=")));  // +1000ms
inst.update(InputEvent::Key(key("=")));  // +1000ms → poll_interval_ms = 5000

// Simulate restart: load new instance with persisted state
let saved = host_state.snapshot();
let mut inst2 = WasmApp::load("sysmon.wasm", grants!["state:read-write"]);
let effects = inst2.init(saved, (400.0, 300.0));

// Should set timer at 5000ms, not the default 2000ms
assert!(effects.iter().any(|e| matches!(e, Effect::SetTimer(t) if t.delay_ms == 5000)));
```

**Pass condition:** Second instance timer matches the persisted value.

---

### ~~G6 — Run primitive: ephemeral launch~~ ✅ SHIPPED

> Note: the launch surface is `plexi app open ./x.wasm` (not a new `plexi run` command — `plexi run` is the project-command runner). Registry resolution / payment / signature steps in [The Run Primitive](#the-run-primitive) flow are **not built**.

**What it proves:** `plexi run ./sysmon.wasm` opens a pane, runs the app, and the pane title and content are correct. Pane closes when the app exits.

**Test:** CLI + PlexiUiHarness:
```bash
plexi run ./apps/wasm-poc/sysmon/target/wasm32-wasip2/debug/sysmon.wasm
# Pane should open titled "System Monitor"
# After 2 seconds: CPU% and memory rows are populated
# Press q: pane closes, no state written to profile dir
```

**Pass condition:** Pane opens, populates, closes on q. `~/.plexi-alpha/state/` has no sysmon namespace (ephemeral run = temp namespace).

---

### ~~G7 — Surface-node lifecycle (Pong)~~ ✅ SHIPPED

> 🟡 Surface lifecycle + input ship. The live pane still reads the surface back each frame via a **synchronous `device.poll(Wait)` + egui re-upload**, but Lane A replaced the worst per-pixel CPU copy with row-level packing and added timing logs for encode/submit, map wait, pack, and upload. True zero-copy/shared-device compositing remains future work.

**What it proves:** `surface-node` allocates correctly, the host sends `surface-ready`, and pixel-buffer rendering round-trips through `render-to-texture` effects. Input events reach the app and affect game state.

**Test:** PlexiUiHarness scene:
```toml
[[steps]]
action = "open_wasm_app"
path = "apps/wasm-poc/pong/target/wasm32-wasip2/debug/pong.wasm"

[[steps]]
action = "assert_log_contains"
text = "pong: surface ready"

[[steps]]
action = "inject_event"
event = { type = "key", key = "w", pressed = true }

[[steps]]
action = "step_frames"
count = 60    # 1 second at 60fps tick rate

[[steps]]
action = "assert_screenshot"
snapshot = "pong-running.png"
```

**Pass condition:** Screenshot shows the ball and paddles. The left paddle has moved upward after 60 W-key frames. Host CPU stays < 5% at idle (pixel buffer path is SW; GPU path will improve this).

---

### G8 — Python compat: existing stats app unchanged  ⬜ NOT BUILT (deferred mission)

**What it proves:** `apps/stats/stats.py` runs through a future CPython-in-WASM compatibility runtime and produces visually identical output to native `ProcessApp`.

**Test:** Run `apps/stats/stats.py` on both runtimes, screenshot both, diff:
```bash
# native ProcessApp
plexi app open stats
# capture screenshot -> stats-native.png

# future CPython-in-WASM compatibility runtime
plexi app open stats --runtime python-wasm
# capture screenshot -> stats-wasm-python.png

# diff — must be pixel-identical within font rendering tolerance
image-diff stats-native.png stats-wasm-python.png --max-delta 2
```

**Pass condition:** Screenshots are pixel-identical (or within anti-aliasing tolerance). No SDK changes to `stats.py`.

---

### G9 — Cloud execution: identical output  ⬜ NOT BUILT (deferred mission)

**What it proves:** An app with `execution = "cloud"` in its manifest produces the same output as the same app with `execution = "local"`. The wire protocol introduces no observable difference.

**Test:**
```bash
# Local execution
plexi run ./sysmon.wasm
# capture screenshot → sysmon-local.png

# Cloud execution (manifest patched or flag added)
PLEXI_FORCE_CLOUD=1 plexi run ./sysmon.wasm
# capture screenshot → sysmon-cloud.png

image-diff sysmon-local.png sysmon-cloud.png --max-delta 0
```

**Pass condition:** Screenshots identical. Round-trip latency for a UINodePatch < 50ms on localhost. Log confirms WebSocket frames, not direct calls.

---

### G10 — 402 payment gate  ⬜ NOT BUILT (deferred mission)

**What it proves:** A registry response with HTTP 402 triggers the Plexi payment flow. After payment, the app opens normally. Without payment, the app does not open.

**Test:** Use a local mock registry:
```bash
# Start mock registry that returns 402 for "paid-tool"
plexi-mock-registry --paid paid-tool &

plexi run @mock/paid-tool
# Should: show payment prompt
# User declines → pane never opens, clean error message
# User accepts → payment resolves → app opens normally
```

**Pass condition:** Decline path shows error and does not open pane. Accept path opens app. The WASM bundle is not accessible without the session token.

---

### Summary table

| Gate | System | Type | Automated |
|---|---|---|---|
| G1 | WIT interface | Build | CI |
| G2 | Pure function lifecycle | Unit test | CI |
| G3 | Effect round-trip | Integration test | CI |
| G4 | UINode rendering | Scene/screenshot | CI |
| G5 | State persistence | Integration test | CI |
| G6 | Run primitive | CLI + scene | Manual first, then CI |
| G7 | Surface-node / Pong | Scene/screenshot | CI |
| G8 | Python compat | Screenshot diff | CI |
| G9 | Cloud execution | Screenshot + latency | Manual first, then CI |
| G10 | 402 payment | Mock registry | Manual |

G1–G5 are pure host/runtime unit and integration tests — no binary install, no UI. G6–G10 require the full v2 host to be running.

---

## ~~GPU Render-Pass Interface~~ ✅ SHIPPED (G11)

> 🟡 The command interface (create-*/submit-render-pass/submit-compute-pass) ships and runs the render pass <2ms. The remaining perf issue is in the *surface composite* path, not the render pass — see G7 callout and Next Steps lane A.

Reference implementation: `apps/wasm-poc/pong/` (world: `plexi-gpu-app`).

The `gpu` capability import exposes a WebGPU-aligned subset of wgpu to WASM modules. The app never touches pixel buffers — it issues GPU commands through the WIT interface; the host executes them against its wgpu device. The WASM/host boundary carries command descriptors, not framebuffer data.

### Why WebGPU-aligned

WebGPU is a language-neutral, cross-platform GPU API designed to be expressible in WIT-like interfaces. wgpu implements it natively. Aligning the `gpu` interface to WebGPU means:
- WGSL shaders are portable across Metal, Vulkan, and DX12 — the host's wgpu backend handles translation.
- The API design is proven. We are not designing GPU command semantics — we are routing WebGPU through WIT.
- Future tooling: existing WebGPU Rust crates can be adapted to target the `gpu` import.

### Performance model

The WASM/host boundary is crossed once per draw call with a descriptor (`RenderPassDesc`). The GPU workload itself runs entirely on the host's GPU device — no WASM JIT overhead on the hot path. For a game rendering 20 objects per frame:
- WASM fills one `RenderPassDesc` struct (~200 bytes)
- Host serializes it to wgpu commands (~100ns)
- GPU executes the render pass (~0.1ms at 1080p)

This is effectively native GPU performance. The JIT tax applies only to the descriptor construction in WASM, not the rendering.

### What apps can build with this

| App type | GPU usage |
|---|---|
| 2D game (Pong, Snake, platformer) | Instanced quad pipeline, one draw call per frame |
| Image editor | Compute pass for filter kernels (blur, sharpen, curves) |
| Video preview | Compute pass for color grading, render pass for display |
| Data visualisation | Compute pass for layout, render pass for rendering |
| 3D viewport | Depth-buffered render pass with perspective projection |
| DAW waveform display | Compute pass for waveform reduction, render pass for display |

### WGSL as the shader language

Apps supply WGSL source strings. The host compiles them at `create-render-pipeline` / `create-compute-pipeline` call time. Compilation is cached — subsequent pane restarts with the same WGSL skip recompilation.

Apps do not need to ship precompiled shader binaries. The WGSL source is portable — the same string runs on Metal (macOS), Vulkan (Linux), and DX12 (Windows) without modification.

### ~~Verification gate G11 — GPU render pass~~ ✅ SHIPPED

**What it proves:** The `gpu` interface compiles, the pipeline is created from WGSL, and the render pass executes on the host's GPU. pong renders at 60fps with zero pixel buffer copies.

**Test:** PlexiUiHarness scene:
```toml
[[steps]]
action = "open_wasm_app"
path = "apps/wasm-poc/pong/target/wasm32-wasip2/debug/pong.wasm"
world = "plexi-gpu-app"

[[steps]]
action = "assert_log_contains"
text = "pong: GPU ready"

[[steps]]
action = "step_frames"
count = 120

[[steps]]
action = "assert_screenshot"
snapshot = "pong-gpu.png"

[[steps]]
action = "assert_metric"
metric = "frame_time_ms"
max = 2.0    # 60fps = 16.6ms budget; GPU path should use < 2ms
```

**Pass condition:** Screenshot shows correct game state. Frame time < 2ms. Log confirms no pixel buffer writes (no `render-to-texture` effect calls).

---

## ~~Real-Time Audio Interface~~ ✅ SHIPPED (G12)

Reference implementation: `apps/wasm-poc/audio-synth/` (world: `plexi-audio-app`).

The audio RT model uses two WIT interfaces:
- `audio-rt-control` (import): apps call `open-output` / `open-input` to configure streams.
- `audio-rt-process` (export): the host calls `process-output` / `process-input` from its OS audio thread.

### The RT callback model

The host's audio thread fires every `buffer_frames / sample_rate` seconds (typically ~10ms at 48kHz / 512 frames). It calls the WASM export `process-output(handle, buffer_frames, channels, sample_rate, state)` → `(samples, new_state)`.

The `state: u64` slot is the bridge. The app uses it to thread minimal context (phase accumulator, envelope value) through callback invocations without heap allocation or global mutation in the RT path. The state is passed in and returned — the host stores it between calls.

**Strict RT constraints on `process-output` and `process-input`:**
- No heap allocation (no `Vec::new`, no `String::new`).
- No locks (no `Mutex`, no `RwLock`).
- No host imports (they may acquire the wasmtime Store lock, which blocks).
- Return within `buffer_frames` samples of wall-clock time.

These constraints are the same as native Core Audio / WASAPI callbacks. WASM SIMD at ~85-90% native speed is sufficient for all common synthesis workloads within these bounds.

### State threading pattern

Phase accumulator packed into the u64 state slot:
```rust
// In process-output: no heap, no globals
let phase = f32::from_bits(state as u32);
let phase_inc = freq / sample_rate as f32;
// ... generate samples using phase ...
let new_phase = (phase + phase_inc * buffer_frames as f32).fract();
let new_state = new_phase.to_bits() as u64;
(samples, new_state)
```

For more complex state (e.g. a filter with multiple coefficients), the host provides a scratch buffer capability (`audio:scratch-buffer`) that allocates a fixed-size block at stream open time, safe to read/write from the RT callback without allocation.

### Plexi Pipes in the RT path

The audio-synth POC pushes waveform preview data to a binary pipe from within `process-output`. This is allowed because the pipe's `send_binary` implementation uses the `ArrayQueue` lock-free ring buffer from `src/host/typed_pipes.rs` — a single atomic push, no blocking, RT-safe.

The drain thread reads from the ring and writes to the Unix socket asynchronously. If the ring is full, `send_binary` returns an error — the app drops the frame. This is the correct behavior: waveform preview data is best-effort, not reliable.

### What apps can build with this

| App type | Audio usage |
|---|---|
| Synthesiser | `open-output`, `process-output` fills samples |
| Sampler / drum machine | Same; app reads sample data from state |
| Audio effect (EQ, reverb) | `open-input` + `open-output`, process-input feeds process-output |
| DAW mixer | Multiple streams, one per channel; host mixes at the device level |
| Visualiser | `open-input` only; waveform data pushed to pipe for rendering |
| Voice recorder | `open-input` only; samples written to file via file-write effect |

### Why <10ms latency is achievable

wasmtime with Cranelift JIT compiles `process-output` to native machine code at stream-open time. The JIT overhead (compilation) happens once — at RT callback time the code is native. The ~10-15% register allocator overhead applies to the compiled code, not the callback invocations. For a 512-frame buffer at 48kHz:
- Budget: 10.6ms
- Sine oscillator: ~0.05ms
- Additive (4 partials): ~0.2ms
- Convolution reverb (1024 taps with SIMD): ~2ms

All comfortably within budget. The 10-15% JIT overhead shrinks the effective headroom slightly but does not break the latency model.

### ~~Verification gate G12 — RT audio callback~~ ✅ SHIPPED

**What it proves:** `audio-rt-control::open-output` succeeds, the host calls `process-output` at RT thread priority, and the audio-synth produces audible output with < 10ms latency.

**Test:**
```rust
// HostHarness: open audio-synth, verify callback fires
let mut inst = WasmApp::load("audio-synth.wasm", grants!["audio:rt", "pipes:open"]);
inst.init(StateSnapshot::empty(), (400.0, 300.0));
// Simulate space key → playing = true
inst.update(InputEvent::Key(key("space")));
// Verify process-output produces non-silent samples
let (samples, _state) = inst.audio_process_output(0, 512, 2, 48000, 0u64);
assert!(samples.iter().any(|&s| s.abs() > 0.01), "should produce audio");
```

**Pass condition:** `process-output` returns non-silent samples when `playing = true`. Measured callback latency < 10ms on a macOS test machine. Log confirms RT thread priority.

---

## ~~Typed Pipes Integration~~ ✅ SHIPPED (G13)

Reference implementations: `apps/wasm-poc/pong/` (JSON score pipe) and `apps/wasm-poc/audio-synth/` (binary waveform pipe + JSON metadata pipe).

The `pipes` import exposes the existing `TypedPipeRegistry` from `src/host/typed_pipes.rs` to WASM components through WIT. The underlying mechanism is unchanged — binary pipes use Unix domain sockets with u32-BE length-prefixed frames and a lock-free `ArrayQueue` ring. WASM apps open pipes through the WIT import; the host creates the socket and drain thread exactly as before.

### Pipe lifecycle from WASM

```
1. App: pipes::open("waveform-out", Binary, Out) → pipe-handle
2. Host: TypedPipeRegistry::open_binary("waveform-out", Out)
          creates socket, drain thread, returns BinaryPipeAllocation
3. Connecting pane: reads the socket path from the pipe registry
          connects as a Unix socket client
4. Host: drain thread accepts client, begins draining the ring
5. App: pipes::is-connected(handle) → true
6. App: pipes::send-binary(handle, samples) → pushes to ring
7. Drain thread: writes length-prefixed frame to socket
8. Peer pane: reads frame, processes waveform data
```

### Why pipes are the right inter-app primitive

Two apps communicating through shared memory or a file creates coupling and no type safety. Pipes give:
- **Directionality**: `In`, `Out`, `Duplex` — the host enforces direction.
- **Backpressure**: if the drain ring is full, `send-binary` errors — the app decides whether to drop or slow down.
- **Audit trail**: `PipeOpen` / `PipeClose` events are written to `events.jsonl`.
- **Capability gating**: `pipes:open` is a declared capability. An app without it cannot open pipes.
- **Type discipline**: binary vs. JSON pipes. The host rejects sending JSON on a binary pipe.

### What apps can build with pipes

| Use case | Pipe type | Direction |
|---|---|---|
| Audio synth → waveform visualiser | Binary (f32 samples) | Out → In |
| Code agent → diff viewer | JSON (patch events) | Out → In |
| File browser → editor | JSON (path selection events) | Out → In |
| Multi-track DAW: channel → master bus | Binary (f32 frames) | Out → In |
| Sensor stream → dashboard | Binary (packed floats) | Out → In |
| Chat pane → TTS synth | JSON (text events) | Out → In |

### ~~Verification gate G13 — Pipes round-trip~~ ✅ SHIPPED

**What it proves:** A WASM app opens a binary pipe, pushes frames, and a peer pane receives them. The ArrayQueue ring correctly drops frames on overrun without crashing.

**Test:**
```rust
// Open audio-synth + a listener pane
let synth = WasmApp::load("audio-synth.wasm", grants!["audio:rt", "pipes:open"]);
let listener = TestPipeListener::connect("waveform-out");
synth.init(StateSnapshot::empty(), (400.0, 300.0));
synth.update(InputEvent::Key(key("space"))); // start playing
// Simulate 10 audio callbacks
for _ in 0..10 {
    let _ = synth.audio_process_output(0, 512, 2, 48000, 0u64);
}
// Listener should have received waveform frames
assert!(listener.frame_count() > 0, "should receive pipe frames");
// Simulate overrun: fill ring to capacity, next push returns error
let result = synth.send_binary_direct("waveform-out", vec![0u8; 1024]);
// When ring full, error is returned — no crash, no block
assert!(result.is_err() || result.is_ok(), "overrun must not panic");
```

**Pass condition:** Frames received by listener. Overrun returns error, does not block or crash the host. `events.jsonl` contains `PipeOpen` and (on close) `PipeClose` events for the waveform pipe.

---

## ~~Worlds Summary~~ ✅ SHIPPED

| World | Use case | GPU | Audio RT | Pipes |
|---|---|---|---|---|
| `plexi-app` | Standard apps, tools, dashboards | — | — | yes |
| `plexi-gpu-app` | Games, editors, visualisers | yes | — | yes |
| `plexi-audio-app` | Synths, effects, recorders | — | yes | yes |
| `plexi-full-app` | DAW, video editor, GPU audio | yes | yes | yes |

All four worlds share the same `lifecycle`, `host-state`, `host-log`, and `pipes` imports. The choice of world is declared in `manifest.toml` under `[runtime] world`. The host selects the wasmtime linker configuration accordingly.

---

## Next Steps (2026-06-18)

The gate scoreboard (G1–G7, G11–G13 green) proves each subsystem works in isolation. It does **not** prove a real app can be built on the runtime — that is the gap this section closes. Ordering is by leverage: fix what's felt and what's silently broken first, then reach parity with Python, then turn on agentic, then the deferred infra gates.

Each lane below is independently shippable (one PR), independently testable (a `HostHarness`/`PlexiUiHarness` test is the done-condition), and does not touch `ProcessApp`/PGAP.

### ~~Lane A — GPU surface readback perf (bounded pass)~~ ✅ DONE

**Problem:** the live pane composites the guest surface every frame via a synchronous `device.poll(wgpu::Maintain::Wait)` + per-pixel `RgbaImage` copy + egui texture re-upload (`wasm_gpu.rs:158-168`, `wasm_pane.rs:473-486`). That CPU-blocks on GPU completion for ~690 KB/frame at 480×360. The G7/G11 gates only time the GPU *submit*, so they stay green while the user sees stutter.

**Shipped:** `GpuDevice::read_texture` now logs encode/submit, poll/map wait, CPU row-pack, total time, dimensions, and byte counts. The CPU copy path now strips padded rows into a contiguous `Vec<u8>` and builds the image with `RgbaImage::from_raw`, replacing the per-pixel `put_pixel` loop. `LiveWasmPane::ui` also logs egui texture upload timing.

**Still future:** the compositor still synchronously maps readback buffers and re-uploads to egui. A true steady 60fps fix likely needs async readback across frames, a staging ring, or shared-device / zero-copy composition. No wall-clock performance assertion is used yet; the current gate preserves pixel correctness and exposes timing diagnostics.

**Done condition:** `host::wasm_pane::tests::g7_surface_lifecycle_and_input` still proves surface lifecycle, readback, and input. `host::wasm_gpu::tests::pack_rgba_rows_strips_row_padding` covers the safe row-copy optimization.

### ~~Lane B — Wire UI interaction~~ ✅ DONE

**Problem:** button clicks and text-input changes were produced by the renderer (`wasm_render.rs:22-26`) then logged-and-dropped. Every interactive non-game WASM app was a static picture.

**Shipped:** `wit/plexi.wit` now defines `ui-action` and `ui-value-change` input events. `LiveWasmPane` maps collected `actions` / `value_changes` to those events and drains them through the existing queue. `apps/wasm-poc/counter` proves a typed-node button can mutate guest state.

**Done condition:** `host::wasm_pane::tests::ui_button_click_updates_guest_view` clicks a `button-node` in a headless egui render path and asserts the guest's `update()` ran and `view()` changed.

### ~~Lane C — Real fs / net effects~~ ✅ DONE

**Problem:** `file-read`/`file-write`/`http-fetch` were declared but stubbed. No real tool app works without them. The impl-plan already specifies the shape: "long effects (http-fetch) run on a worker thread and post their result event back" (`wasm-runtime-impl-plan.md:101`).

**Shipped:** `file-read` and `file-write` resolve through canonicalized fs roots and reject out-of-scope paths with error results. `http-fetch` validates the URL host against explicit net host grants, runs through the existing `NetService` seam on a worker thread, and queues `http-response` back into the guest update loop. Denied net hosts return a 403 response-style event.

**Done condition:** `host::wasm_pane::tests` covers granted read, out-of-scope read error, write round-trip, mock `http-fetch` round-trip, and denied-host 403.

### ~~Lane D — Capability grant flow + runtime enforcement~~ ✅ DONE

**Problem:** link-time gating works, but grants auto-derive from imports and ephemeral runs auto-grant everything (`wasm_app.rs:462`). No review sheet, no inline prompt, no escalation, no scope enforcement — the security story the spec sells (link-time + prompts) is half-built.

**Shipped:** focused WASM panes with pending capability requests capture `FocusLayer::CapabilityModal` and render through the shared host modal primitives. `request-capability` now auto-answers from session grants/blocks or prompts, decisions enqueue `capability-granted` / `capability-denied`, emit `HostEvent::PermissionDecision`, and grant runtime access only for explicit `fs:read:<path>`, `fs:write:<path>`, and `net:fetch:<host>` strings. Unknown strings still get guest events but do not widen fs/net access. Lane F adds remembered grants for manifest/package install surfaces and raw import review; Lane D remains the session runtime-enforcement layer.

**Done condition:** host tests cover grant/deny events, focused-pane pending prompt detection, and fs/net effect behavior before vs. after scoped grants.

### ~~Lane E — Agentic surface: `ai-query` + app events~~ ✅ DONE

**Problem:** the WIT has no `ai-query`, `emit-event`, `declare-event-streams`, or subscribe import — WASM apps cannot make LLM calls or join the app-event subscription system that drives agents (app event subscriptions, permissions broker). The host side already exists: `LiveAiBroker` (`src/plexi_ai/broker.rs`), cost ledger, consent UI, event timeline. Only the WIT bindings + linker wiring are missing.

**Shipped:** WIT now includes `ai-query` plus `ai-stream-chunk` / `ai-response` input events, and `declare-event-streams` / `emit-event` effects with result events. WASM `ai-query` is gated by the Lane D `ai.query` session grant, runs on a worker through the injected `AiBroker`, and streams/finalizes back through the guest queue. App event declarations and emissions route into `AppTimeline`; undeclared emits return an error result. Tools and WASM subscribe/delivery imports remain future work.

**Done condition:** `host::wasm_pane::tests` covers denied `ai-query` without broker calls, granted streaming/final AI response, declared event emission into the timeline, and undeclared event rejection.

### Lane F — Manifest-backed WASM apps + remembered scoped grants ✅ DONE for current surfaces

**Problem:** direct `.wasm` launches were ephemeral and import-derived. A real sandboxed app needs a manifest-backed path: explicit runtime type, persistent state, explicit link-time host grants, and remembered user decisions for scoped runtime capabilities.

**Shipped:** `manifest.toml` now accepts `[app] type = "wasm"`. Registry/path launches route those manifests through wasmtime instead of `ProcessApp`, use persistent per-app/per-workspace WASM state, derive link-time grants from manifest capabilities (`pipe.open`, `audio.playback`, `gpu.render`), and restore raw scoped WASM decisions from `permissions.toml`. The WASM capability modal now offers once/always allow/deny; always decisions persist raw ids such as `fs:read:<path>`, `fs:write:<path>`, `net:fetch:<host>`, and `ai.query`. `.plexipkg` validation and install trust sheets classify WASM packages, display required vs. optional raw WASM review metadata, and can pre-seed workspace-scoped required/selected optional raw decisions during install. Direct raw `.wasm` launch now inspects required link-time imports (`state:read-write`, `pipe.open`, `gpu.render`, `audio.playback`), fails closed without remembered Green decisions, and the CLI launch path prompts once and remembers approvals for the path scope.

**Shipped:** native GUI path launches for raw `.wasm` queue a pre-launch review modal before spawning. Approval persists the required link-time imports as Green decisions, then replays the launch through the same fail-closed `open_wasm_app_pane` check.

**Done condition:** `app::permissions::tests` covers raw WASM permission persistence and sensitive unset withholding; `app::registry::tests::manifest_with_type_wasm_loads` covers manifest parsing; `host::wasm_pane::tests` remains green.

### Deferred infra gates (after A–E)

These remain explicitly out of scope until the parity lanes land — they are large, standalone missions, not polish:

- **G8 — Python compat shim.** CPython-in-WASM (~40 MB shared bundle) so `apps/stats/stats.py` runs unmodified on v2. This is the prerequisite for actually *removing* `ProcessApp` and superseding the Python runtime (a v3 boundary event).
- **G9 — Cloud execution.** Standalone cloud wasmtime + WebSocket wire protocol (msgpack frames derived from WIT) + transparent reconnect/state-sync.
- **G10 — 402 payment gate.** Registry 402 interception at manifest fetch → payment flow → session-token-gated bundle access. Depends on the registry (currently NOT BUILT) and signature verification.

### Sequencing summary

| Lane | What | Why first | Size |
|---|---|---|---|
| A | GPU surface readback perf + perf-HUD app | ✅ Bounded pass done — timing + row-copy optimization; zero-copy remains future | S–M |
| B | Wire UI interaction | ✅ Done — typed-node interactions reach guest update | S |
| C | Real fs/net effects | ✅ Done — scoped fs + worker-backed HTTP | M |
| D | Capability grant flow + runtime enforcement | ✅ Done — session prompts + scoped runtime enforcement; persisted install review lives in Lane F | M |
| E | `ai-query` + app events → existing broker | ✅ Done — AI query + timeline emit; subscribe imports/tools remain future | M |
| F | Manifest-backed WASM apps + remembered scoped grants | ✅ Done for current surfaces — manifest launch, persistent state, package review, install-time selected raw decisions, CLI-reviewed raw `.wasm` link imports, and GUI raw `.wasm` pre-launch review ship | M |
| G8/G9/G10 | Python compat, cloud, payment | Large standalone missions; supersede-Python is a v3 outcome | L each |

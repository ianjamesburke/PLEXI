# Plexi App Framework + Marketplace PRM

Status: active.
Stint: 0008-0031 (sprint graph in `.stint/`).

This PRM defines the path from "Plexi can run plugins" to "Plexi is a plugin platform." For v1 it owns plugin authoring, plugin packaging, marketplace trust, hosted marketplace install, paid-plugin planning, and Plexi AI subscription planning.

PGAP/Python and WASM are parallel plugin runtimes under one plugin contract. PGAP remains the native, simple authoring path for reviewed local plugins. WASM is the sandbox and performance path for plugins that need stronger isolation, typed host imports, or direct GPU surfaces. MCPUI interop, `Surface` polish, and Bevy remain v2 lanes that fit the same plugin contract; they do not block the v1 release.

For those areas, this file supersedes older roadmap fragments, SDK overhaul plans, marketplace notes, and MCPUI future-enhancement docs. Superseded docs should be removed instead of kept as parallel history.

## Purpose

Finish Plexi as a platform in the order that makes the product usable and defensible:

1. Agents can generate good Plexi plugins on the first try.
2. App permissions and packages are clear enough that a user can make an informed install decision.
3. Local package/install works before hosted marketplace work.
4. Hosted registry, paid plugins, revenue share, and Plexi AI subscription arrive after the local plugin framework is stable.
5. PGAP/Python and WASM are supported as different runtimes under one plugin contract; MCPUI and Bevy fit beside them instead of replacing them.

The local-first rule stays intact. Installed plugins and user data live on disk. Hosted services may sell plugins, review submissions, and broker AI calls, but they must not become required for running installed plugins.

## Product Decisions

- PGAP remains Plexi's native plugin protocol.
- WASM remains a first-class sandbox/performance runtime, not a forced replacement for PGAP.
- SDK v3 is the canonical authoring path. Plugins expose module-level `init(size, args)`, `update(event)`, and `view()` functions. Normal plugins return L1 UI from `view()`; games and realtime canvases return `Canvas(...)` from `view()` and update from `RenderFrame` events.
- `Raw` stays as an escape hatch. It is not the default path for generated plugins.
- MCPUI is a v2 interop lane. First export Plexi plugins as MCPUI resources. Later host MCPUI plugins in Plexi through WebView panes.
- WASM is the third-party sandbox and performance runtime. It shares packaging, trust labels, capability review, and plugin identity with PGAP plugins.
- Bevy targets WASM + `Surface` in v2. Native Bevy embedding is not the first path.
- Marketplace trust cannot launch while plugins have ambient host control through inherited environment and CLI subprocesses.
- Python plugins run through the CPython-in-WASM adapter, sharing the WASM runtime's packaging, trust labels, and component sandbox (stint 0285). Trust labels reflect review provenance, not sandbox strength — see `docs/wasm-runtime.md`.
- Marketplace and Plexi AI subscription are business surfaces, but they do not block local package/install or plugin-framework completion.

## Definition Of Finished

The plugin framework is finished when:

- `plexi plugin init` creates a plugin that uses `view()` and L1 components by default.
- `sdk/python/SDK_V3.md` plus the scaffold template are enough for an agent to build a working plugin without reading Rust.
- Core plugins are clean references for common patterns: list, form, text edit, table/data, network fetch, AI chat, state persistence, and canvas escape hatch.
- Normal plugins do not hand-place pixels.
- Canvas and game plugins use `Canvas(...)` plus `RenderFrame` intentionally and are labeled that way in docs and tests.
- `TextEdit` works as a normal component in the tree.
- App authoring tests prove generated plugins render, handle input, save state, and avoid layout overlap at small and normal pane sizes.

The trust and packaging foundation is finished when:

- App manifests declare the host powers the plugin can actually use.
- Plugins cannot use inherited host routing or CLI subprocesses to bypass plugin identity and capability checks.
- Assistant declares every pane/app/terminal capability it uses, and those powers go through host-mediated plugin APIs.
- Packages can be validated before install.
- Package validation checks manifest fields, runtime, capabilities, package contents, generated metadata, and obvious bypass patterns.
- Install screens show runtime trust labels and declared capabilities before the user proceeds.
- Install screens show a review-provenance trust label and declared capabilities; every plugin launches through the enforced wasmtime component boundary with scoped grants (`docs/wasm-runtime.md`).

The marketplace plan is finished when:

- A user can install a free local plugin package.
- A publisher can validate a package before submission.
- A reviewed plugin shows capabilities, runtime type, and trust labels before install.
- Hosted registry, publisher accounts, submission review, paid plugins, revenue share, and Plexi AI subscription are specified.
- Remote registry and payment work are not prerequisites for the local plugin framework.

## Implementation Plan

### 1. Finish App Authoring

Make one path obvious:

- Keep module-level `view()` as the only render entrypoint.
- Keep `Canvas(...)` as the explicit canvas/realtime path inside `view()`.
- Reject legacy `App` subclasses in `plexi plugin check`.
- Make the scaffold short, stateful, and visually correct.
- Keep `plexi view render`, `plexi pane state`, `plexi pane key`, and `plexi pane action` as the agent drive loop: render, inspect, act, inspect again.
- Move normal Core plugins to `view()` + L1 UI.
- Keep games and realtime visual plugins on the canvas path, with plugin docs saying why.
- Make `TextEdit` usable inside normal component trees.
- Add focused plugin authoring tests for scaffold, input, state, TextEdit, small-pane layout, and canvas fallback.

The first milestone is "an agent can generate a good local Plexi app." Hosted marketplace infrastructure (registry standup, CDN) may proceed in parallel — it depends on the package metadata format, not on finished authoring. The authoring milestone gates what gets *published* (the review quality bar), not when registry work starts.

### 2. Clean Up Permissions And Trust

The current permission model protects host APIs, not the plugin's own in-sandbox code. That is acceptable only if Plexi says so and routes host powers through the same model.

Required work:

- Stop passing ambient host control to plugin processes, or bind it to plugin identity and capability checks.
- Treat `PLEXI_SOCKET` as host routing, not a plugin permission.
- Replace Assistant-style CLI subprocess control tools with host-mediated PGAP/tool APIs before any plugin becomes a trusted marketplace surface.
- Add explicit capabilities for every pane/app/terminal power a PGAP assistant-style plugin can use. Reuse `panes.spawn` and `terminal.bindings` where they fit; add narrower capabilities where they do not.
- Make capability declarations match actual powers before marketplace trust labels ship.
- Keep `docs/wasm-runtime.md`'s "Security Model" section (stint 0409) aligned with the actual sandbox and trust-label behavior as it evolves.
- Add denial tests for host APIs reachable by plugins.

Trust labels must be blunt:

- `Reviewed`: a reviewed Python or WASM app. Review and manifest are trust aids layered on the sandbox, not the isolation boundary itself — see `docs/wasm-runtime.md`.
- `Sandboxed WASM`: plugin runs under the WASM/WASI runtime with scoped host grants. Do not show this before it is true.
- `First-party core`: bundled with Plexi and maintained in this repo.

### 3. Define Packages And Local Install

A Plexi plugin package is:

- `manifest.toml`
- plugin source and assets
- generated package metadata
- file list and checksums
- declared runtime, SDK/protocol requirements, and capabilities
- trust label inputs
- optional publisher metadata

Package validation must fail on:

- missing or invalid `manifest.toml`
- unknown capability strings
- runtime not supported by the installed Plexi build
- entry file missing
- package files outside the plugin root
- symlinks or path traversal that escape the package root
- generated metadata that does not match package contents
- obvious bypass patterns in reviewed-native packages, such as subprocess or socket use, unless the package is labeled and reviewed for those powers

Ship local package/import first:

- `plexi plugin package <path>` creates a package artifact.
- `plexi plugin validate <path-or-package>` validates the package contract.
- `plexi plugin install <path-or-package>` installs locally after showing manifest, runtime, trust label, and capabilities.

The exact command names can change during implementation, but the user workflow cannot: validate, inspect, install, run locally.

### 4. Add Marketplace

Hosted marketplace work starts after local packages are useful. The detailed S4 spec lives in [`marketplace-hosted.md`](marketplace-hosted.md), covering tasks `0018`-`0022`.

Marketplace surfaces:

- hosted plugin registry
- publisher accounts
- package submission
- automated validation
- human review for native-process plugins
- trust labels and capability display
- plugin install from registry
- update metadata
- paid plugins
- revenue share
- refunds and takedowns
- publisher analytics that do not expose user data
- Plexi AI subscription for `ai.query`

Business model:

- Free plugins can install from local packages or hosted registry.
- Paid plugins use hosted purchase and license metadata, but installed code and user state still live on disk.
- Plexi AI subscription is separate from plugin purchase. Plugins call `ai.query`; the host decides whether that routes to local Ollama, user-owned OpenRouter keys, or a Plexi-managed subscription backend.
- The AI subscription can offer a free request allowance before requiring payment, but that number belongs in the billing spec, not the plugin framework.

Do not make hosted login required for local plugins.

### 5. v1 Release Readiness

Before release readiness, run a UI stabilization sprint on top of the completed Host UI Kit sequence:

- Audit host chrome for one-off modal shells, raw shortcut labels, ad hoc permission prompts, and package/install confirmation UI.
- Refactor remaining v1 modals, shortcuts, permission grants, package trust sheets, and marketplace install chrome onto shared UI primitives.
- Use the host UI gallery and focused regression tests to keep states visible: normal, hover-equivalent, selected, focused, disabled, danger, permission-required, and trust-warning.

Then, before cutting v1:

- Remove superseded docs instead of keeping parallel history.
- Regenerate public docs and CLI references from the current build.
- Reconcile open GitHub issues against stint tasks and v1/v2 labels.
- Verify install, upgrade, channel isolation, local package install, hosted marketplace install, and trust-label wording.
- Keep `docs/wasm-runtime.md`'s "Security Model" section, `sdk/python/SDK_V3.md`, website docs, and README aligned.

### Runtime Lanes

PGAP/Python remains the simple path:

- best for small tools, agents, local workflows, and first-party references
- fastest path for AI-generated plugins
- reviewed native process trust label until WASM exists

WASM is the sandbox/performance path:

- best for untrusted third-party code, games, realtime media, and portable non-Python plugins
- typed host imports and link-time capability gating
- sandboxed WASM trust label when the plugin uses the enforced WASM runtime path

Raw/canvas remains the escape hatch:

- games
- animation
- custom visualizations
- realtime media

MCPUI lane:

- first: export Plexi plugins as MCPUI resources through the MCP bridge
- later: host MCPUI plugins in Plexi using the WebView pane contract in [`browser-surface.md`](browser-surface.md)
- keep PGAP and MCPUI parallel; do not contort PGAP into HTML

WASM/WASI lane:

- keep expanding host-effect parity and typed imports
- map Plexi capabilities to WASI grants where WASI is introduced
- use the same plugin manifest, package trust labels, and install review flow
- keep the same L1 UI contract where possible

Bevy lane:

- implement `Surface` before serious Bevy work
- target Bevy to WASM + `Surface`
- do not start with native Bevy embedding

## Test Plan

SDK tests:

- scaffolded view renders through `view()`
- scaffolded plugin handles key input
- scaffolded plugin persists state
- L1 component events dispatch correctly
- `TextEdit` works inside a normal component tree
- small-pane rendering has no text overlap or footer clipping
- canvas plugins render through `Canvas(...)` returned from `view()`
- a plugin that subclasses legacy `App` fails loudly

Permission tests:

- plugin without `panes.spawn` cannot spawn panes through host APIs
- plugin without `terminal.bindings` cannot drive terminal bindings
- plugin without plugin-opening capability cannot open another plugin through host APIs
- assistant-style plugin manifests declare the capabilities used by their tools
- socket-based host commands cannot bypass plugin identity checks
- inherited environment does not grant host control to a plugin

Packaging tests:

- valid plugin package installs locally
- missing manifest fails validation
- malformed manifest fails validation
- unknown capability fails validation
- unsupported runtime fails validation
- package with path traversal fails validation
- package with symlink escape fails validation
- package trust label reflects runtime and review state

Marketplace acceptance scenarios:

- user installs a free local plugin package
- publisher validates a package before submission
- reviewed plugin displays capabilities and a review-provenance trust label before install
- sandboxed WASM plugin displays sandboxed trust label only after WASM runtime enforcement exists
- paid-plugin purchase and license flow is specified but does not block local package/install
- Plexi AI subscription is specified as an `ai.query` backend, not as a requirement for local plugins

## Source-Of-Truth Rules

- This PRM owns v1 planning for plugin authoring, trust, packaging, marketplace, paid-plugin planning, Plexi AI subscription planning, and release readiness.
- Runtime lanes after v1 are parked here as v2 direction until their own PRM or stint sprint is created.
- `sdk/python/SDK_V3.md` is the SDK API reference. It must remain consistent with this PRM.

- `docs/SECURITY_MODEL.md` never existed as a file (this PRM's pointer was aspirational). `docs/wasm-runtime.md`'s "Security Model" section (stint 0409) is now the canonical security/capability-model reference: Python plugins are sandboxed (stint 0285, CPython-in-WASM), not native unsandboxed processes.
- Superseded plans under `docs/superpowers/plans/` and `docs/superpowers/specs/` should be removed when they conflict with the current PRM.
- The old MCPUI standalone plan has been removed. This PRM owns MCPUI sequence.
- This PRM resolves conflicts for plugin framework and marketplace decisions. Sprint tasks live in `.stint/`.

When a future issue changes a decision here, update this PRM in the same PR.

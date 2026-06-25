# Plexi App Framework + Marketplace PRM

Status: active.
Stint: 0008-0031 (sprint graph in `.stint/`).

This PRM defines the path from "Plexi can run apps" to "Plexi is an app platform." For v1 it owns app authoring, app packaging, marketplace trust, hosted marketplace install, paid-app planning, and Plexi AI subscription planning.

PGAP/Python and WASM are parallel app runtimes under one app contract. PGAP remains the native, simple authoring path for reviewed local apps. WASM is the sandbox and performance path for apps that need stronger isolation, typed host imports, or direct GPU surfaces. MCPUI interop, `Surface` polish, and Bevy remain v2 lanes that fit the same app contract; they do not block the v1 release.

For those areas, this file supersedes older roadmap fragments, SDK overhaul plans, marketplace notes, and MCPUI future-enhancement docs. Superseded docs should be removed instead of kept as parallel history.

## Purpose

Finish Plexi as a platform in the order that makes the product usable and defensible:

1. Agents can generate good Plexi apps on the first try.
2. App permissions and packages are clear enough that a user can make an informed install decision.
3. Local package/install works before hosted marketplace work.
4. Hosted registry, paid apps, revenue share, and Plexi AI subscription arrive after the local app framework is stable.
5. PGAP/Python and WASM are supported as different runtimes under one app contract; MCPUI and Bevy fit beside them instead of replacing them.

The local-first rule stays intact. Installed apps and user data live on disk. Hosted services may sell apps, review submissions, and broker AI calls, but they must not become required for running installed apps.

## Product Decisions

- PGAP remains Plexi's native app protocol.
- WASM remains a first-class sandbox/performance runtime, not a forced replacement for PGAP.
- SDK v3 is the canonical authoring path. Apps expose module-level `init(size, args)`, `update(event)`, and `view()` functions. Normal apps return L1 UI from `view()`; games and realtime canvases return `Canvas(...)` from `view()` and update from `RenderFrame` events.
- `Raw` stays as an escape hatch. It is not the default path for generated apps.
- MCPUI is a v2 interop lane. First export Plexi apps as MCPUI resources. Later host MCPUI apps in Plexi through WebView panes.
- WASM is the third-party sandbox and performance runtime. It shares packaging, trust labels, capability review, and app identity with PGAP apps.
- Bevy targets WASM + `Surface` in v2. Native Bevy embedding is not the first path.
- Marketplace trust cannot launch while apps have ambient host control through inherited environment and CLI subprocesses.
- Python marketplace apps are reviewed native processes until WASM ships. Do not describe them as sandboxed.
- Marketplace and Plexi AI subscription are business surfaces, but they do not block local package/install or app-framework completion.

## Definition Of Finished

The app framework is finished when:

- `plexi app init` creates an app that uses `view()` and L1 components by default.
- `sdk/python/SDK_V3.md` plus the scaffold template are enough for an agent to build a working app without reading Rust.
- Core apps are clean references for common patterns: list, form, text edit, table/data, network fetch, AI chat, state persistence, and canvas escape hatch.
- Normal apps do not hand-place pixels.
- Canvas and game apps use `Canvas(...)` plus `RenderFrame` intentionally and are labeled that way in docs and tests.
- `TextEdit` works as a normal component in the tree.
- App authoring tests prove generated apps render, handle input, save state, and avoid layout overlap at small and normal pane sizes.

The trust and packaging foundation is finished when:

- App manifests declare the host powers the app can actually use.
- Apps cannot use inherited host routing or CLI subprocesses to bypass app identity and capability checks.
- Assistant declares every pane/app/terminal capability it uses, and those powers go through host-mediated app APIs.
- Packages can be validated before install.
- Package validation checks manifest fields, runtime, capabilities, package contents, generated metadata, and obvious bypass patterns.
- Install screens show runtime trust labels and declared capabilities before the user proceeds.
- Python apps are labeled as reviewed native processes. WASM apps are labeled as sandboxed when they run through the enforced WASM runtime path with scoped grants.

The marketplace plan is finished when:

- A user can install a free local app package.
- A publisher can validate a package before submission.
- A reviewed app shows capabilities, runtime type, and trust labels before install.
- Hosted registry, publisher accounts, submission review, paid apps, revenue share, and Plexi AI subscription are specified.
- Remote registry and payment work are not prerequisites for the local app framework.

## Implementation Plan

### 1. Finish App Authoring

Make one path obvious:

- Keep module-level `view()` as the only render entrypoint.
- Keep `Canvas(...)` as the explicit canvas/realtime path inside `view()`.
- Reject legacy `App` subclasses in `plexi app check`.
- Make the scaffold short, stateful, and visually correct.
- Keep `plexi app render`, `plexi pane state`, `plexi pane key`, and `plexi app action` as the agent drive loop: render, inspect, act, inspect again.
- Move normal Core apps to `view()` + L1 UI.
- Keep games and realtime visual apps on the canvas path, with app docs saying why.
- Make `TextEdit` usable inside normal component trees.
- Add focused app authoring tests for scaffold, input, state, TextEdit, small-pane layout, and canvas fallback.

The first milestone is "an agent can generate a good local Plexi app." Hosted marketplace infrastructure (registry standup, CDN) may proceed in parallel — it depends on the package metadata format, not on finished authoring. The authoring milestone gates what gets *published* (the review quality bar), not when registry work starts.

### 2. Clean Up Permissions And Trust

The current permission model protects host APIs, not the native Python process. That is acceptable only if Plexi says so and routes host powers through the same model.

Required work:

- Stop passing ambient host control to app processes, or bind it to app identity and capability checks.
- Treat `PLEXI_SOCKET` as host routing, not an app permission.
- Replace Assistant-style CLI subprocess control tools with host-mediated PGAP/tool APIs before any app becomes a trusted marketplace surface.
- Add explicit capabilities for every pane/app/terminal power a PGAP assistant-style app can use. Reuse `panes.spawn` and `terminal.bindings` where they fit; add narrower capabilities where they do not.
- Make capability declarations match actual powers before marketplace trust labels ship.
- Keep `docs/SECURITY_MODEL.md` honest: Python apps are native processes with consent + audit.
- Add denial tests for host APIs reachable by apps.

Trust labels must be blunt:

- `Reviewed native process`: Python or other native app. Review and manifest are trust aids, not isolation.
- `Sandboxed WASM`: app runs under the WASM/WASI runtime with scoped host grants. Do not show this before it is true.
- `First-party core`: bundled with Plexi and maintained in this repo.

### 3. Define Packages And Local Install

A Plexi app package is:

- `manifest.toml`
- app source and assets
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
- package files outside the app root
- symlinks or path traversal that escape the package root
- generated metadata that does not match package contents
- obvious bypass patterns in reviewed-native packages, such as subprocess or socket use, unless the package is labeled and reviewed for those powers

Ship local package/import first:

- `plexi app package <path>` creates a package artifact.
- `plexi app validate <path-or-package>` validates the package contract.
- `plexi app install <path-or-package>` installs locally after showing manifest, runtime, trust label, and capabilities.

The exact command names can change during implementation, but the user workflow cannot: validate, inspect, install, run locally.

### 4. Add Marketplace

Hosted marketplace work starts after local packages are useful. The detailed S4 spec lives in [`marketplace-hosted.md`](marketplace-hosted.md), covering tasks `0018`-`0022`.

Marketplace surfaces:

- hosted app registry
- publisher accounts
- package submission
- automated validation
- human review for native-process apps
- trust labels and capability display
- app install from registry
- update metadata
- paid apps
- revenue share
- refunds and takedowns
- publisher analytics that do not expose user data
- Plexi AI subscription for `ai.query`

Business model:

- Free apps can install from local packages or hosted registry.
- Paid apps use hosted purchase and license metadata, but installed code and user state still live on disk.
- Plexi AI subscription is separate from app purchase. Apps call `ai.query`; the host decides whether that routes to local Ollama, user-owned OpenRouter keys, or a Plexi-managed subscription backend.
- The AI subscription can offer a free request allowance before requiring payment, but that number belongs in the billing spec, not the app framework.

Do not make hosted login required for local apps.

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
- Keep `docs/SECURITY_MODEL.md`, `sdk/python/SDK_V3.md`, website docs, and README aligned.

### Runtime Lanes

PGAP/Python remains the simple path:

- best for small tools, agents, local workflows, and first-party references
- fastest path for AI-generated apps
- reviewed native process trust label until WASM exists

WASM is the sandbox/performance path:

- best for untrusted third-party code, games, realtime media, and portable non-Python apps
- typed host imports and link-time capability gating
- sandboxed WASM trust label when the app uses the enforced WASM runtime path

Raw/canvas remains the escape hatch:

- games
- animation
- custom visualizations
- realtime media

MCPUI lane:

- first: export Plexi apps as MCPUI resources through the MCP bridge
- later: host MCPUI apps in Plexi using WebView panes
- keep PGAP and MCPUI parallel; do not contort PGAP into HTML

WASM/WASI lane:

- keep expanding host-effect parity and typed imports
- map Plexi capabilities to WASI grants where WASI is introduced
- use the same app manifest, package trust labels, and install review flow
- keep the same L1 UI contract where possible

Bevy lane:

- implement `Surface` before serious Bevy work
- target Bevy to WASM + `Surface`
- do not start with native Bevy embedding

## Test Plan

SDK tests:

- scaffolded app renders through `view()`
- scaffolded app handles key input
- scaffolded app persists state
- L1 component events dispatch correctly
- `TextEdit` works inside a normal component tree
- small-pane rendering has no text overlap or footer clipping
- canvas apps render through `Canvas(...)` returned from `view()`
- an app that subclasses legacy `App` fails loudly

Permission tests:

- app without `panes.spawn` cannot spawn panes through host APIs
- app without `terminal.bindings` cannot drive terminal bindings
- app without app-opening capability cannot open another app through host APIs
- assistant-style app manifests declare the capabilities used by their tools
- socket-based host commands cannot bypass app identity checks
- inherited environment does not grant host control to an app

Packaging tests:

- valid app package installs locally
- missing manifest fails validation
- malformed manifest fails validation
- unknown capability fails validation
- unsupported runtime fails validation
- package with path traversal fails validation
- package with symlink escape fails validation
- package trust label reflects runtime and review state

Marketplace acceptance scenarios:

- user installs a free local app package
- publisher validates a package before submission
- reviewed native app displays capabilities and native-process trust label before install
- sandboxed WASM app displays sandboxed trust label only after WASM runtime enforcement exists
- paid-app purchase and license flow is specified but does not block local package/install
- Plexi AI subscription is specified as an `ai.query` backend, not as a requirement for local apps

## Source-Of-Truth Rules

- This PRM owns v1 planning for app authoring, trust, packaging, marketplace, paid-app planning, Plexi AI subscription planning, and release readiness.
- Runtime lanes after v1 are parked here as v2 direction until their own PRM or stint sprint is created.
- `sdk/python/SDK_V3.md` is the SDK API reference. It must remain consistent with this PRM.

- `docs/SECURITY_MODEL.md` remains the current security disclosure as long as it says Python apps are not sandboxed.
- Superseded plans under `docs/superpowers/plans/` and `docs/superpowers/specs/` should be removed when they conflict with the current PRM.
- The old MCPUI standalone plan has been removed. This PRM owns MCPUI sequence.
- This PRM resolves conflicts for app framework and marketplace decisions. Sprint tasks live in `.stint/`.

When a future issue changes a decision here, update this PRM in the same PR.

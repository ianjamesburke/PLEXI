# Plexi App Framework + Marketplace PRM

Status: canonical planning source for the v1 app-platform release path.
Last updated: 2026-06-10.

This PRM defines the path from "Plexi can run apps" to "Plexi is an app platform." For v1 it owns app authoring, app packaging, marketplace trust, hosted marketplace install, paid-app planning, and Plexi AI subscription planning.

MCPUI interop, WASM/WASI, `Surface`, and Bevy are v2 runtime lanes. They must fit the same app contract, but they do not block the v1 release.

For those areas, this file supersedes older roadmap fragments, SDK overhaul plans, marketplace notes, and MCPUI future-enhancement docs. Superseded docs should be removed instead of kept as parallel history.

## Purpose

Finish Plexi as a platform in the order that makes the product usable and defensible:

1. Agents can generate good Plexi apps on the first try.
2. App permissions and packages are clear enough that a user can make an informed install decision.
3. Local package/install works before hosted marketplace work.
4. Hosted registry, paid apps, revenue share, and Plexi AI subscription arrive after the local app framework is stable.
5. MCPUI, WASM/WASI, and Bevy are planned as v2 runtime lanes that fit under the same app contract instead of replacing it.

The local-first rule stays intact. Installed apps and user data live on disk. Hosted services may sell apps, review submissions, and broker AI calls, but they must not become required for running installed apps.

## Sprint Plan

The operational sprint graph lives in `.stint/`. Use `stint status`, `stint next`, and `stint sprint show <id>` for the active task list and blockers.

GitHub issues are still useful implementation tickets while `.stint` stabilizes, but `.stint` is the sprint graph. A stint task may link zero, one, or many GitHub issues. When the two disagree, update the task first, then reconcile issue labels or bodies during issue hygiene.

| Sprint | Goal | Task range |
|---|---|---|
| S1 | File Explorer becomes a Host UI Kit based daily-driver file surface. | `0001`-`0007` |
| S2 | App authoring path is clear enough for Core 9 and third-party package authors. | `0008`-`0012` |
| S3 | Packages install locally with explicit capability and trust handling. | `0013`-`0017` |
| S4 | Hosted Marketplace can list and install reviewed apps. | `0018`-`0022` |
| S5 | Host UI stabilization: centralize v1 modals, shortcuts, permission grants, and app-platform chrome on the new UI kit. | `0023`-`0027` |
| S6 | v1 release readiness: docs, issue hygiene, install QA, and security wording are clean enough to cut v1. | `0028`-`0031` |

S1 is the File Explorer sprint. The File Explorer issue bundle is linked from `docs/prm/file-explorer-overhaul.md`.

### Sprint Tasks

| Task | Sprint | Work |
|---|---|---|
| `0001`-`0007` | S1 | File Explorer overhaul: adaptive list/details layout, columns, inspector/Quick Look, safe file operations, recursive search, richer views, Plexi-native selection actions. |
| `0008` | S2 | Polish scaffold and app dev defaults so generated apps start from `view()` and L1 UI. |
| `0009` | S2 | Standardize app-author dev loop: init, health, test, lint, render, inspect, act. |
| `0010` | S2 | Polish SDK components and small-pane behavior. |
| `0011` | S2 | Sweep Core 9 apps into clean references for common app patterns. |
| `0012` | S2 | Add app-authoring verification harness and docs. |
| `0013` | S3 | Remove or identity-bind ambient host control inherited by app processes. |
| `0014` | S3 | Move Assistant pane/app/terminal powers through host-mediated capability APIs. |
| `0015` | S3 | Define package artifacts and validator contract. |
| `0016` | S3 | Add local install inspection and trust sheet. |
| `0017` | S3 | Add permission management and yellow-state routing needed for package trust. |
| `0018` | S4 | Stand up hosted app registry/CDN for reviewed app metadata. |
| `0019` | S4 | Add publisher submission and review flow. |
| `0020` | S4 | Browse and install reviewed apps from registry. |
| `0021` | S4 | Specify paid apps, licenses, revenue share, refunds, takedowns, and analytics. |
| `0022` | S4 | Specify Plexi AI subscription as an `ai.query` backend. |
| `0023` | S5 | Audit remaining host chrome and identify one-off modal, shortcut, permission, and trust UI paths. |
| `0024` | S5 | Move remaining modals and shortcut hint surfaces onto the centralized Host UI Kit. |
| `0025` | S5 | Rework permission grant, package trust, and install confirmation popups on shared UI primitives. |
| `0026` | S5 | Normalize keyboard shortcut display and command/help affordances across host chrome. |
| `0027` | S5 | Add UI regression coverage and gallery states for v1 host/app-platform chrome. |
| `0028` | S6 | Purge stale docs and regenerate public docs for v1. |
| `0029` | S6 | Reconcile open GitHub issues with stint sprints, labels, and v1/v2 boundaries. |
| `0030` | S6 | Run install, upgrade, channel, package, and marketplace acceptance QA. |
| `0031` | S6 | Audit security/trust wording so v1 never claims Python sandboxing. |

## Current Truth

These are code facts as of 2026-06-09. Re-check before starting an implementation issue.

- PGAP is Plexi's native app protocol. Apps still speak newline-delimited JSON over stdin/stdout, with binary payloads on typed pipes.
- `UiNode` exists in `src/protocol/ui_nodes.rs`. It includes L0 primitives, L1 components, `Raw`, `TextEdit`, and the reserved `Surface` node.
- SDK v2 is partly landed. `App.view()` dispatch exists in `sdk/python/plexi_sdk/_app.py`, and the Python scaffold template uses `view()` and `self.state`.
- The canonical app-authoring docs are `docs/sdk-v2.md` and `docs/SDK_QUICKSTART.md`.
- `Surface { id }` exists only as a placeholder. `src/render/components.rs` treats it as a no-op.
- `TextEdit` exists as a host-rendered `UiNode`, but the Python wrapper still documents it as a `ctx.render_tree(...)` node rather than a normal component-tree child.
- Python apps are native subprocesses. They are not sandboxed. Capabilities gate PGAP host APIs, not Python's direct access to filesystem, network, subprocesses, environment variables, or local IPC.
- Local install exists. `plexi app install <path>` copies an app directory into the channel app store, and the top-level install flow can install git sources and packs.
- App validation exists but is shallow. It is not yet a marketplace package validator.
- `plexi app publish` is a stub.
- The Assistant app declares only `ai.query` in `apps/assistant/manifest.toml`, but registers tools that call the Plexi CLI in subprocesses to open terminals, open apps, list panes, and send pane commands. Those subprocesses inherit the app environment, including host routing such as `PLEXI_SOCKET`.
- MCPUI is not implemented in the runtime. It is a v2 runtime lane.
- WASM/WASI is not an app runtime yet. It is a v2 runtime lane.
- Bevy has no first implementation path until `Surface` and the WASM lane exist. It is a v2 runtime lane.

## Product Decisions

- PGAP remains Plexi's native app protocol.
- SDK v2 is the canonical authoring path. A normal app implements `view()` and returns L1 UI. `on_render(ctx)` is for games, realtime canvases, visualizations, and other explicit pixel-control apps.
- `Raw` stays as an escape hatch. It is not the default path for generated apps.
- MCPUI is a v2 interop lane. First export Plexi apps as MCPUI resources. Later host MCPUI apps in Plexi through WebView panes.
- WASM/WASI is the v2 third-party sandbox and performance lane.
- Bevy targets WASM + `Surface` in v2. Native Bevy embedding is not the first path.
- Marketplace trust cannot launch while apps have ambient host control through inherited environment and CLI subprocesses.
- Python marketplace apps are reviewed native processes until WASM ships. Do not describe them as sandboxed.
- Marketplace and Plexi AI subscription are business surfaces, but they do not block local package/install or app-framework completion.

## Definition Of Finished

The app framework is finished when:

- `plexi app init` creates an app that uses `view()` and L1 components by default.
- `docs/sdk-v2.md` plus the scaffold template are enough for an agent to build a working app without reading Rust.
- Core apps are clean references for common patterns: list, form, text edit, table/data, network fetch, AI chat, state persistence, and canvas escape hatch.
- Normal apps do not hand-place pixels.
- Canvas and game apps use `on_render(ctx)` intentionally and are labeled that way in docs and tests.
- `TextEdit` works as a normal component in the tree.
- App authoring tests prove generated apps render, handle input, save state, and avoid layout overlap at small and normal pane sizes.

The trust and packaging foundation is finished when:

- App manifests declare the host powers the app can actually use.
- Apps cannot use inherited host routing or CLI subprocesses to bypass app identity and capability checks.
- Assistant declares every pane/app/terminal capability it uses, and those powers go through host-mediated app APIs.
- Packages can be validated before install.
- Package validation checks manifest fields, runtime, capabilities, package contents, generated metadata, and obvious bypass patterns.
- Install screens show runtime trust labels and declared capabilities before the user proceeds.
- Python apps are labeled as reviewed native processes; WASM apps are labeled as sandboxed only after the WASM runtime exists.

The marketplace plan is finished when:

- A user can install a free local app package.
- A publisher can validate a package before submission.
- A reviewed app shows capabilities and trust labels before install.
- Hosted registry, publisher accounts, submission review, paid apps, revenue share, and Plexi AI subscription are specified.
- Remote registry and payment work are not prerequisites for the local app framework.

## Implementation Plan

### 1. Finish App Authoring

Make one path obvious:

- Keep `view()` as the default hook for normal apps.
- Keep `on_render(ctx)` for explicit canvas/realtime apps.
- Keep "never override both" as a hard SDK rule.
- Make the scaffold short, stateful, and visually correct.
- Keep `plexi app render`, `plexi pane state`, `plexi pane key`, and `plexi app action` as the agent drive loop: render, inspect, act, inspect again.
- Move normal Core apps to `view()` + L1 UI.
- Keep games and realtime visual apps on the canvas path, with app docs saying why.
- Make `TextEdit` usable inside normal component trees.
- Add focused app authoring tests for scaffold, input, state, TextEdit, small-pane layout, and canvas fallback.

The first milestone is "an agent can generate a good local Plexi app." Do not start hosted marketplace work before this is true.

### 2. Clean Up Permissions And Trust

The current permission model protects host APIs, not the native Python process. That is acceptable only if Plexi says so and routes host powers through the same model.

Required work:

- Stop passing ambient host control to app processes, or bind it to app identity and capability checks.
- Treat `PLEXI_SOCKET` as host routing, not an app permission.
- Replace Assistant's CLI subprocess control tools with host-mediated PGAP/tool APIs.
- Add explicit capabilities for every pane/app/terminal power the Assistant can use. Reuse `panes.spawn` and `terminal.bindings` where they fit; add narrower capabilities where they do not.
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

Hosted marketplace work starts after local packages are useful.

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
- Keep `docs/SECURITY_MODEL.md`, `docs/PGAP_REFERENCE.md`, `docs/sdk-v2.md`, website docs, and README aligned.

### v2 Runtime Lanes

PGAP/Python remains the simple path:

- best for small tools, agents, local workflows, and first-party references
- fastest path for AI-generated apps
- reviewed native process trust label until WASM exists

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

- add WASM runtime before claiming strong sandboxing for third-party paid apps
- map Plexi capabilities to WASI grants
- use the same app manifest and package trust labels
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
- legacy canvas apps still render through `on_render(ctx)`
- an app that overrides both `view()` and `on_render(ctx)` fails loudly

Permission tests:

- app without `panes.spawn` cannot spawn panes through host APIs
- app without `terminal.bindings` cannot drive terminal bindings
- app without app-opening capability cannot open another app through host APIs
- Assistant's manifest declares the capabilities used by its tools
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
- `docs/sdk-v2.md` remains the SDK API reference as long as it matches this PRM.
- `docs/PGAP_REFERENCE.md` remains the wire reference as long as it matches code.
- `docs/SECURITY_MODEL.md` remains the current security disclosure as long as it says Python apps are not sandboxed.
- Superseded plans under `docs/superpowers/plans/` and `docs/superpowers/specs/` should be removed when they conflict with the current PRM.
- The old MCPUI standalone plan has been removed. This PRM owns MCPUI sequence.
- `ROADMAP.md` can summarize milestones, but this PRM resolves conflicts for app framework and marketplace decisions.

When a future issue changes a decision here, update this PRM in the same PR.

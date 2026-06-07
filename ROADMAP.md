# Plexi Roadmap

Sequential execution plan. Each layer assumes the previous is stable. Check off items as their issues close. See [NORTH_STAR.md](NORTH_STAR.md) for the vision; this is the tactical plan.

No dates. Order matters, dates don't. Pivot when necessary, but respect dependencies.

---

## Layer 1: Stabilize the Bones

The P0 refactors that make everything else cheaper to build. Every subsequent feature touches keybindings, modals, CLI commands, or focus. Building on unfixed infra means building twice.

- [x] Declarative keybinding table (#1916) -- eliminate subset-match footgun, order dependence, 4-site new-binding tax (v0.0.601)
- [x] Modal shell helper (#1915) -- extract scrim+Area+Frame into reusable `modal_shell`, kill copy-paste across 7 overlay files (v0.0.601)
- [x] CLI pane spawning unification (#1923) -- `pane new` as single entry point for terminal spawning
- [x] CLI namespace clean split (#1928) -- remove `--app`/`--mcp` overlap from `pane new`, separate `app open` fully (v0.0.599)
- [x] CLI direction flag fix (#1927) -- `pane new` direction flags produce wrong split orientation (v0.0.599)
- [ ] ~~FocusLayer + PlexiInput unification (#1238, #1239)~~ -- deferred to v2.0 (current FocusLayer stack works; unification is architectural ideal, not v1 blocker)
- [x] App viewport overtake (#1924) -- Escape-to-return pane navigation stack (v0.0.601)

**Done when:** all P0s closed, `cargo test --bin plexi` green, no known input-routing regressions.

### 1b: Codebase structure (do alongside Layer 1)

When a refactor PR already touches a file, move it into the right module in the same PR. No standalone reorganization sprint.

- [x] CLI module split -- `src/cli.rs` refactored into `src/cli/` submodules (#1870)
- [x] App module split -- `src/app/mod.rs` refactored into `src/app/` submodules (#1865)
- [ ] Semantic module reorganization -- group remaining loose `src/*.rs` files into `src/ui/` (widgets, style, render_components, sidebar, minimap, tiling), `src/io/` (audio, video, midi, typed_pipes), `src/state/` (context, workspace, secrets, event_log)
- [x] Split `app_protocol.rs` into `protocol/` submodules (#2044) (v0.0.634)
- [ ] Extract reusable UI patterns from overlays into `src/ui/widgets/` (text inputs, scrollable lists, search bars, confirmation dialogs)

---

## Layer 2: Polish the Surfaces

Make what exists feel finished before adding new systems. These are the first things a new user touches.

- [x] ~~Welcome screen redesign (#1575)~~ -- closed as stale; welcome screen already matches proposed design
- [x] Text editor extraction (#1920, #1922) -- native text-editor builtin pane shipped as `src/text_editor_app.rs`
- [x] Terminal Cmd+F search overlay (#1914) -- match cycling, keyboard-navigable (v0.0.610)
- [x] Auto-set pane title to wrapped command (#1037) -- auto-titles for --mcp and --cli panes (v0.0.610)
- [x] Notification auto-dismiss when originating pane focused (#1635) (v0.0.603)
- [x] QuickNote modal blocked by other modals (#1626) (v0.0.603)
- [x] URL trailing punctuation fix (#1549) (v0.0.599)
- [x] ~~Install modal success detection (#1643)~~ -- closed, install flow sufficient for v1
- [x] Core app theming audit (#1669) -- ctx.theme tokens across 7 apps (v0.0.604)
- [x] ~~QuickNote destination overhaul (#1622)~~ -- closed, Cmd+0 host feature covers this
- [x] QuickNote (Cmd+0) modal closes on click-away (#1938) (v0.0.610)
- [x] Portal minimap real-time activity (#1918) -- shipped in `ca2c3e83` (v0.0.597), closed

**Done when:** a new user can install, see a polished welcome screen, open apps that look consistent, use QuickNote without hitting modal bugs.

---

## Layer 3: Lock the Protocol

The biggest and most important layer. PGAP becomes L1-only declarative UI. This is the "100 lines of Python, ChatGPT gets it right" milestone.

### ⚠️ 3-OVERHAUL: SDK v2 Overhaul (TOP PRIORITY — do before anything else in Layer 3)

The Python SDK has two render paths, ctx leaking into event handlers, a broken scaffold template, and no canonical state API. An agent reading the template today cannot produce a correct app. Fix this before any new apps ship.

**Full plan:** `docs/superpowers/plans/2026-06-07-sdk-overhaul.md`

- [ ] Emergency template fix — broken layout, invisible footer (`divider=False` bug)
- [ ] `App.state` property — `self.state.get/save` replaces `ctx.load_state/save_state`
- [ ] `view()` as primary hook — apps return component tree, SDK dispatches it
- [ ] `on_init(self)` ctx-free dispatch — no ctx needed at init time
- [ ] Rewrite scaffold template with v2 API
- [ ] Migrate all Core 9 apps to v2 API
- [ ] `plexi pane key <id> <key>` CLI command — agent dev loop (Playwright for apps)
- [ ] `docs/sdk-v2.md` — golden developer reference

**Done when:** An agent can read `docs/sdk-v2.md` + the `app_init` template and produce a working, visually correct Plexi app on the first try. All Core 9 apps use the v2 pattern.

---

### 3a: Protocol redesign

Current state: `UiNode` enum has 21 variants. L1 layout components (AppBar, FooterKeys, Footer, Section, Label, Spacer, Divider, Card, SelectList) added in v0.0.606. Host-side renderers for all L1 variants live in `render_components.rs`. SDK `to_node()` enables automatic L1 tree emission with L0 fallback for custom canvas components.

- [x] Design the full L1 UiNode set -- expanded from 12 to 21 variants: AppBar, FooterKeys, Footer, Section, Label, Spacer, Divider, Card, SelectList (v0.0.606). Chat node deferred to Layer 4 (agent app).
- [x] Remove `_l0` fallback fields from L1 nodes (Button, Input, Badge, Dot) (v0.0.603)
- [x] Deprecate and remove L0 flat draw commands as the primary rendering path -- `render_tree()` now emits UiNode trees via `to_node()` when all children support it; `Raw { command }` stays as escape hatch for custom canvas (v0.0.606)
- [x] Host-side L1 renderer -- `render_component_tree()` handles all 21 UiNode variants with consistent spacing, colors, and focus management (v0.0.606)
- [x] Component event routing (#1904) -- host fires `ComponentEvent` for interactive L1 nodes; SDK `on_component_event` hook dispatches to apps (v0.0.609)

### 3b: SDK rewrite

- [x] Rewrite Python SDK to emit UiNode trees instead of L0 draw commands -- `to_node()` on all standard components, `render_tree()` auto-selects L1 path (v0.0.606)
- [x] SDK becomes a tree builder: `ctx.render(Column([AppBar(...), SelectList(...), FooterKeys(...)]))` emits UiNode tree (v0.0.606)
- [x] SDK actionable error messages (#1203) -- 17 guarded entry points, 39 new tests (v0.0.610)
- [ ] Layout fundamentals (#1527) -- headline alignment, character padding handled by host, not SDK
- [x] Default Esc behavior in base App class (#1631) (v0.0.599)
- [ ] Text selection in PGAP apps (#1645) -- host handles selection for L1 Text nodes
- [ ] PGAP TextEdit node -- host-rendered text editor primitive, apps can embed it for editable content

### 3c: Core 9 migration + app init

The **Core 9** ship with the default install. Everything else moves to the marketplace as first-party published apps:

1. **Assistant** (AI chat agent, the flagship, built in Layer 4)
2. **Backlog** (workspace task management, integrates with QuickNote)
3. **Todo** (simple list, the "hello world that's useful")
4. **Calculator** (universal utility, button interaction)
5. **Logs** (host log viewer, audit trail visibility)
6. **CSV Viewer** (file data tool, demonstrates fs capability)
7. **Wikipedia** (search + read, demonstrates net.http capability)
8. **Stats** (usage dashboard, unique to Plexi, local-first data)
9. **Quick Note** (markdown notes, workspace integration)

Current app inventory: 18 shipped apps in `apps/`, 29 POC apps in `apps/dev/`.

- [x] Migrate Core 9 + 4 non-core apps from L0 to L1 Component rendering (v0.0.597) -- backlog, calc, calendar, csv_viewer, gh-projects, kanban, logs, quick-note, todo, typing-tutor, wikipedia
- [x] Migrate canvas/game apps to L1 escape hatch pattern (v0.0.598) -- balls, snake, tetris, stats wrapped in Column([AppBar, Canvas, FooterKeys])
- [x] Move non-core apps to `apps/dev/` (v0.0.598) -- kanban, calendar, gh-projects, mind-map, typing-tutor, counter-tree moved; shipped set is 8 core + 3 games
- [ ] Each core app becomes a reference implementation demonstrating specific L1 patterns
- [ ] Backlog: integrate TextEdit node for inline editing
- [x] Logs: search/filter (#1649) + ~~spacing (#1648)~~ (v0.0.603) -- level filter, target/app_id filter, text search (v0.0.606)
- [x] Update `plexi app init` scaffold to produce a perfect 30-line L1 example (v0.0.599); rewritten as comprehensive SDK reference (v0.0.633)
- [x] Remove Quick Note app from `apps/` (#1943) -- redundant with Cmd+0 host QuickNote feature (v0.0.610)
- [x] Archive apps/dev/ POCs (#1942) -- 31 removed, kept ai-test, counter-tree, pixel-art-tavern (v0.0.610)
- [x] ~~`plexi app dev` hot-reload command (#1660)~~ -- superseded by app init rework: `app init` + `app open <path>` replaces dev workflow (v0.0.633)

### 3d: Documentation

- [x] PGAP protocol reference (#1950) -- `docs/PGAP_REFERENCE.md`, every L1 node type documented with examples (v0.0.633)
- [ ] SDK quickstart -- "your first app in 50 lines"
- [ ] Acknowledge security model in protocol docs: capability system is consent + audit, not process isolation. WASM runtime (Phase 4) is the true sandbox endgame.

**Done when:** an LLM can generate a working, visually polished Plexi app from the SDK reference alone. All shipped apps use L1 exclusively. Protocol version bumped and schema frozen for v1.

---

## Layer 4: Add Intelligence

The agent experience and AI onboarding. The LLM backend infrastructure is substantially built: `src/plexi_ai/` contains broker (37k lines), ledger (15k), tool dispatch (18k), loop (5k), with OpenRouter + Ollama backends, capability-gated `ai.query`, streaming, cost tracking. The SDK already exposes `ai_query()` with timeouts, streaming, and cancellation. What's missing is the user-facing agent experience.

### 4a: Agent as a PGAP app (the "assistant" app)

The agent is a first-party PGAP app, not a new pane type. This means other people can build their own agent apps with different personalities, system prompts, and tool sets. "Characters" are just different agent app manifests. Currently only dev POCs exist (`apps/dev/ai-test/`, `apps/dev/pixel-art-tavern/`).

- [x] Core assistant app (#2041) -- L1 chat UI, multi-turn conversation, streaming via `AiStreamChunk`, model switcher, thinking animation (v0.0.638)
- [ ] Session persistence -- conversations saved as plain JSON in workspace `.plexi/` directory, resume on reopen
- [x] Tool integration (#2037) -- assistant exposes `ExposeTools` so it can invoke tools from other running apps (v0.0.638)
- [x] CLI tool access (#2040) -- assistant spawns panes, opens apps, reads context via tool dispatch (v0.0.638)
- [ ] Agent app template -- `plexi agent init` scaffold with system prompt, tool declarations, and conversation loop boilerplate

### 4b: AI onboarding (adapted from Odysseus hwfit)

- [x] `plexi ai doctor` CLI command -- hardware scan, GPU detection, VRAM measurement, model recommendation (v0.0.638)
- [x] `plexi doctor` capability audit (#1346) -- audits installed apps against config, `--json` for scripting (v0.0.638)
- [x] Local model setup wizard -- `plexi ai setup` interactive Ollama configuration (v0.0.638)
- [ ] OpenRouter setup flow -- guided API key entry, test connection
- [ ] Plexi AI subscription backend (future) -- `PlexiCloudBackend` implementing `AiBackend`, hits Plexi's API proxy. 50 free requests, then paid tier. `BillingModel::Subscription` variant already exists in the codebase.
- [ ] Auto-discovery of running local LLM servers -- port scan for Ollama/vLLM/LM Studio (adapted from Odysseus model_discovery)

### 4c: External agent control surface

Any coding agent (Claude Code, Codex, PI, etc.) can drive Plexi and its apps via CLI. `PaneCmd` now has: New, Name, SetTitle, List, Focus, Close, Send, Self, Info, Capture, Key, Command, State. `AppCmd` has Action. Remaining gaps:

- [x] `plexi pane command <id> <text>` -- send text with optional `--enter` flag (v0.0.638)
- [x] `plexi pane state <id>` -- returns L1 UiNode tree as JSON for app panes, status object for terminals (v0.0.638)
- [x] `plexi app action <id> <action> [args]` (#1980/#2038) -- semantic actions on app panes via `send_app_action` (v0.0.638)
- [ ] App launch arguments (#1638) -- infrastructure for passing args when opening an app
- [ ] Prefix-based open namespace (#1529) -- `cli:`, `mcp:`, `app:` with dynamic completions and crawl cache
- [ ] Unify `plexi run` with scripts directory (#1321) -- global scope scripting surface for agents
- [ ] AI stress-test dev app -- exercises ai.query, streaming, model tiers, cost ledger, tool dispatch, billing models. Lives in `apps/dev/`, not core.

### 4d: Agent ecosystem

- [x] Workspace agent registry -- `plexi agent init/add/update/list` with scoped memory and logs (v0.0.638)
- [ ] Crew dispatch dashboard app (#1456) -- agent-agnostic dispatch with issue picker and pane monitoring
- [ ] Agent marketplace category -- agent apps are just apps with `ai.query` capability; marketplace tags them as "agents"
- [x] Example agent apps (#2036) -- coding, research, writing agent apps shipped (v0.0.638)
- [ ] Agent-to-agent communication -- apps with `ExposeTools` can offer tools to any agent app in the same workspace

**Done when:** a non-technical user runs `plexi ai doctor`, gets set up with a local or cloud model, opens the assistant app, has a conversation, and the assistant can open apps and terminals on their behalf. Cost tracked, transcript on disk, conversation resumable.

---

## Layer 5: Pane Lifecycle

The backpack, the inventory, and pane management beyond the tiling grid.

- [ ] Pane-level hiding (#1948) -- Cmd+H toggles hidden state: outline dots, dimmed tabs, eye icon flash
- [ ] Context-level parking (#1949) -- Cmd+Shift+H parks context into collapsed "Parked (N)" sidebar section
- [ ] Inventory overlay (Cmd+I) -- searchable, keyboard-navigable list of all hidden/parked panes
- [ ] Notifications from hidden panes -- background panes can still fire notifications that surface in the active layout
- [ ] Restore-to-layout -- pull a hidden pane back into the tiling grid (split into focused pane or new window)
- [ ] Context operations -- move pane between contexts (#1634), new-child-context shortcut (#1833)
- [ ] Live miniature rendering for portal tiles (#1495) -- interactive preview of SubContext portal tiles
- [ ] Pane activity detection -- visual indicator for panes with new output since last focus

**Done when:** you can hide a pane, keep working, see its notification, pull it back. The backpack feels natural.

---

## Layer 6: Curate the Garden

Marketplace app preparation and file browser upgrade. Core 9 are already migrated in Layer 3. This layer polishes the remaining apps for marketplace launch.

- [ ] File browser v2 -- preview pane for text files, multi-select, rename/delete/move, text-editor integration (open file -> text editor pane)
- [ ] GH Projects refactor + L1 migration (#1857) -- marketplace-ready
- [ ] Bluesky overhaul (#1646) + avatar fix (#1659) -- marketplace-ready
- [ ] Calendar polish -- marketplace-ready
- [ ] Kanban polish or merge with GH Projects -- decide which survives
- [ ] Mind Map polish -- marketplace-ready
- [ ] Games (Snake, Tetris, Balls, Typing Tutor, Snake Race) -- marketplace showcase apps
- [ ] `plexi app publish` command -- package + upload to registry
- [ ] Publish all non-core apps to marketplace under first-party account (gives marketplace immediate content at launch)
- [ ] Video player POC validation (#1566)

**Done when:** Core 9 in default install are perfect. 20+ additional apps are published to marketplace. File browser is a real tool. `app publish` works end-to-end.

---

## Layer 7: Open the Gates

Marketplace and distribution. Only after the garden is curated (Layer 6).

- [ ] Marketplace backend -- app registry, hosted (Railway or similar)
- [ ] `plexi app install <name>` from remote registry
- [ ] Submission + review flow -- how apps get listed, approval process
- [ ] Revenue sharing model -- app authors earn from paid apps
- [ ] Plexi AI subscription service -- API proxy server, account system, 50 free requests then paid tier. The business model: apps are free (or paid through marketplace), AI is the subscription
- [ ] Website refresh (#1171)
- [x] Onboarding experience -- `plexi demo` interactive keybinding tutorial (v0.0.638)
- [ ] Windows support (#1735, general)
- [ ] Install experience polish across macOS + Windows + Linux
- [ ] `ARCHITECTURE.md` -- module map, data flow diagram, input dispatch pipeline. Only write this once the architecture is stable (post-Layer 3). Add a CLAUDE.md rule to update it when architecture changes.
- [ ] `CONTRIBUTING.md` -- how to add a keybinding, overlay, CLI command, L1 node type, PGAP app. Write after architecture is documented.

**Done when:** someone can discover, install, and use a third-party app. Authors can publish and get paid. Non-technical users can sign up for Plexi AI and have `ai.query` work out of the box with zero configuration. Plexi installs cleanly on all three platforms.

---

## Post-v1: Infrastructure Cleanup

- [ ] Native Rust CLI renderer (#1947) -- replace Python cli-renderer with egui builtin, add help caching, subcommand crawl, Plexi descriptor flag detection
- [ ] Native Rust MCP renderer -- same treatment for mcp-renderer, eliminate Python app dependency for `--mcp` flow
- [ ] Remove `apps/dev/cli-renderer/` and `apps/dev/mcp-renderer/` once builtins ship

---

## Future: Cloud-Hosted Instances (Phase 5 in North Star)

Detailed here for context, not part of the v1 execution plan.

**Vision:** A Plexi instance runs as a server (local, cloud VM, rented GPU). A thin client connects from any machine and renders the UI. Detach from one machine, attach from another. Config, apps, dotfiles, agent transcripts travel with the instance.

**Architecture sketch:**
- Daemon process owns all pane processes (PTY, PGAP, agent loops)
- GUI becomes a rendering client that connects over a local socket or network
- CLI commands route through the daemon (extends existing PLEXI_SOCKET pattern)
- SpacetimeDB as persistence/sync layer for instance state
- Rented compute: spin up a GPU box for video rendering, connect your client, work, disconnect
- Self-hosting: same daemon, same client, just running on your own hardware

**Depends on:** stable host internals (Layer 1), agent pane (Layer 4), pane lifecycle (Layer 5). This is a major architectural refactor that should only happen after the product is proven on the single-machine model.

**Open questions:**
- Can SpacetimeDB host the full pane process tree, or is it better as a state-sync layer with a separate process host?
- Latency budget for remote rendering (terminal input must feel instant)
- Secret management across machines (Keychain is local; cloud instances need a different secret store)
- Billing model for cloud compute

---

## How to use this file

- **Dispatch next:** look at the first unchecked layer. Its items are the dispatch queue.
- **Check off:** when an item's issue is closed and merged to alpha, check it off.
- **Pivot:** if priorities shift, reorder items within a layer. Don't reorder layers without revisiting dependencies.
- **Add:** new items go into the layer where they fit. If they don't fit, they might be a new layer or they belong in GitHub issues, not here.

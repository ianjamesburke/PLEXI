# Plexi Roadmap

Sequential execution plan. Each layer assumes the previous is stable. Check off items as their issues close. See [NORTH_STAR.md](NORTH_STAR.md) for the vision; this is the tactical plan.

For Layers 3-7 app-framework, marketplace, MCPUI, WASM, and Bevy sequencing, [`docs/prm/app-framework-marketplace.md`](docs/prm/app-framework-marketplace.md) is the canonical PRM. This roadmap stays useful as a progress list, but the PRM resolves conflicts.

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

## Layer 3: Finish App Authoring

Source of truth: [`docs/prm/app-framework-marketplace.md`](docs/prm/app-framework-marketplace.md).

Make SDK v2 the only blessed path for normal app authoring. Generated apps use `view()` and L1 components. `on_render(ctx)` stays for canvas, games, animations, and realtime visualizations.

- [ ] Core apps serve as clean references for lists, forms, text edit, tables, network fetches, AI chat, state persistence, and canvas fallback.
- [ ] `TextEdit` works as a normal component-tree child.
- [ ] App authoring tests cover scaffold render, input, state persistence, TextEdit, small-pane layout, and canvas fallback.
- [ ] `docs/sdk-v2.md`, `docs/SDK_QUICKSTART.md`, README, and website app docs all teach the same `view()`-first pattern.

**Done when:** an agent can read the SDK docs and scaffold template, then generate a working, visually correct local Plexi app on the first try.

---

## Layer 4: Permissions And Trust

Make app manifests match the powers apps can actually use.

- [ ] Remove or scope ambient host control through inherited environment and CLI subprocesses.
- [ ] Route Assistant pane/app/terminal powers through host-mediated app APIs.
- [ ] Assistant declares every pane/app/terminal capability its tools use.
- [ ] Denial tests cover app access to pane spawning, terminal control, app opening, and socket-routed host commands.
- [ ] Trust labels distinguish first-party core apps, reviewed native processes, and future sandboxed WASM apps.

**Done when:** marketplace trust labels can say exactly what an installed app is allowed to do, without hiding Python's native-process trust gap.

---

## Layer 5: Package And Local Install

Ship local package validation and install before hosted marketplace work.

- [ ] Define package metadata: manifest, source/assets, file list, checksums, runtime, SDK/protocol requirements, capabilities, and trust-label inputs.
- [ ] Package validation fails missing manifests, malformed manifests, unknown capabilities, unsupported runtimes, path traversal, symlink escapes, and mismatched metadata.
- [ ] Local install shows manifest, runtime, capabilities, and trust label before install.
- [ ] A user can install a free local package and run it without hosted login.

**Done when:** a publisher can validate a package and a user can inspect and install it locally.

---

## Layer 6: Hosted Marketplace

Add hosted distribution after local packages are useful.

- [ ] Hosted app registry.
- [ ] Publisher accounts and submission flow.
- [ ] Automated validation plus human review for native-process apps.
- [ ] `plexi app publish` packages and submits to the registry.
- [ ] `plexi app install <name>` installs from the remote registry.
- [ ] Paid apps, revenue share, refunds, takedowns, and publisher analytics.
- [ ] Plexi AI subscription as an `ai.query` backend, separate from app purchase.

**Done when:** users can discover, inspect, install, and pay for third-party apps without giving up local ownership of installed code or state.

---

## Layer 7: Runtime Lanes

Keep Python/PGAP simple, add interop and sandbox lanes under the same app contract.

- [ ] Export Plexi apps as MCPUI resources before hosting MCPUI apps in WebView panes.
- [ ] Add WASM/WASI runtime before claiming strong sandboxing for third-party paid apps.
- [ ] Map Plexi capabilities to WASI grants.
- [ ] Implement `Surface` before serious Bevy/game-engine work.
- [ ] Target Bevy to WASM + `Surface`, not native embedding first.

**Done when:** Python, MCPUI, WASM, and Bevy paths share the same manifest/package/trust model.

---

## Parallel Track: Pane Lifecycle

The backpack, the inventory, and pane management beyond the tiling grid. This is still valid product work, but it is not part of the app-framework marketplace PRM.

- [ ] Pane-level hiding (#1948) -- Cmd+H toggles hidden state: outline dots, dimmed tabs, eye icon flash
- [ ] Context-level parking (#1949) -- Cmd+Shift+H parks context into collapsed "Parked (N)" sidebar section
- [ ] Inventory overlay (Cmd+I) -- searchable, keyboard-navigable list of all hidden/parked panes
- [ ] Notifications from hidden panes -- background panes can still fire notifications that surface in the active layout
- [ ] Restore-to-layout -- pull a hidden pane back into the tiling grid (split into focused pane or new window)
- [ ] Context operations -- move pane between contexts (#1634), new-child-context shortcut (#1833)
- [ ] Live miniature rendering for portal tiles (#1495) -- interactive preview of SubContext portal tiles
- [ ] Pane activity detection -- visual indicator for panes with new output since last focus

**Done when:** you can hide a pane, keep working, see its notification, pull it back.

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

# Plexi 2.0 — Release Scope

> ## PROTECTED SPEC — DO NOT EDIT WITHOUT CAUSE
>
> This document is the single source of truth for **what ships in Plexi 2.0 and what does not**. Every item here is either in-scope or explicitly deferred.
>
> **If you must edit:**
> 1. The edit must be approved by the repo owner (enforced via `CODEOWNERS`).
> 2. Every edit must bump `Last updated` at the top of the file.
> 3. Every edit must add a `[DECISION]` entry in `DEV_LOG.md` explaining what changed and why.
> 4. Scope additions require a paired edit to the technical contract or a referenced sub-spec/roadmap. Scope additions to this file alone are not valid.
> 5. Checklist status updates are the only edits that do not need all four steps above. They still need a DEV_LOG entry when an item flips to done.

**Status:** Draft
**Last updated:** 2026-04-16
**Owner:** plexi-core (ianjamesburke)
**Release target:** Plexi 2.0 — recursive agent-native foundation

---

## What Plexi 2.0 Is

Plexi 2.0 is **the recursive agent-native release**.

The foundational unit is no longer "an app in a pane." The foundational unit is a `.plexi` directory: a sealed instance boundary with its own canvas, apps, agents, permissions, events, and state. The directory tree becomes Plexi's depth tree. Panes still split across X/Y; `.plexi` boundaries add the Z-axis.

Every nested Plexi instance is just another app speaking PGAP. It is spawned as a subprocess, communicates through newline-delimited JSON over stdin/stdout, and receives only the capabilities its parent grants. The same protocol path supports human UI, app composition, and agent delegation.

The existing v2 primitives stay in scope because they are the support structure for recursion:

- `OpenIntent` explains why a depth/app was opened.
- The event bus records depth transitions, app lifecycle, pipe writes, notifications, and runs.
- Runs track multi-step work inside a depth.
- Rich notifications jump back to the depth that emitted them.
- Capability enforcement makes nested instances sealed boxes instead of polite conventions.
- Typed pipes compose apps within and across depths without inventing arbitrary RPC.
- Plexi IQ becomes the root/depth-aware orchestrator rather than a standalone agent feature.

The explicit design constraint: **recursion must be visible before it is complete.** The first v2 proof must visualize `.plexi` subdirectories and let the user move through depth without requiring embedded rendering, portals, or direct pipe promotion to be finished.

---

## Source-of-Truth Index

This doc indexes scope. The linked files define the details.

| Role | File | What it defines |
|---|---|---|
| Compass | [`docs/VISION.md`](../../VISION.md) | Why Plexi exists. The non-negotiables every v2 decision is checked against. |
| Spec index | [`docs/specs/README.md`](../README.md) | Canonical map of release contracts, subsystems, proposals, and roadmaps. |
| v2 technical contract | [`docs/specs/releases/plexi-v2.0.md`](plexi-v2.0.md) | The v2 wire format, scope rationale, ship order, and validation rules. |
| Fractal subsystem | [`docs/specs/subsystems/fractal-pgap.md`](../subsystems/fractal-pgap.md) | Recursive `.plexi` instance model, depth navigation, portals, capability containers. |
| Fractal roadmap | [`docs/specs/roadmaps/fractal-pgap/`](../roadmaps/fractal-pgap/) | Agent-sized implementation specs for the recursive proof of concept. |
| App infrastructure | [`docs/specs/subsystems/app-infrastructure.md`](../subsystems/app-infrastructure.md) | v1 app protocol foundation that v2 extends. |
| Typed pipes | [`docs/specs/subsystems/typed-pipes.md`](../subsystems/typed-pipes.md) | Typed app composition; Phase 1 ships in v2. |
| Agent orchestration | [`docs/specs/subsystems/agent-orchestration.md`](../subsystems/agent-orchestration.md) | IQ/delegation rationale; trust floats remain deferred. |
| Agent mode UI | [`docs/specs/subsystems/agent-mode.md`](../subsystems/agent-mode.md) | Per-pane Ctrl+/ surface that v2 wires into IQ. |
| Manifest schema | [`schemas/plexi-manifest-schema.json`](../../../schemas/plexi-manifest-schema.json) | App manifest validation. |

---

## In Scope for 2.0 — Authoritative List

1. **Protocol version negotiation** — nested instances can safely speak across version boundaries.
2. **`.plexi` boundary discovery + depth tree proof of concept** — the host discovers nested `.plexi` directories and renders them as navigable depth nodes.
3. **Process lifecycle foundation** — child process groups, graceful shutdown, forced cleanup, `Suspend`, and `Resume`.
4. **Render summary protocol** — `RenderMode`, `StatusSummary`, and cheap preview/status data.
5. **Embedded Plexi spike** — `plexi --embedded` proves or disproves PGAP-rendered nested Plexi instances without an OS window.
6. **Host event bus** — append-only event stream for app lifecycle, depth transitions, notifications, pipe writes, permissions, and runs.
7. **Tree status rollup** — root can accumulate a registry of active depths and child status.
8. **`OpenIntent` on `Init`** — launches carry file/prompt/caller/run/depth context through the protocol.
9. **Run primitive** — depth-scoped multi-step task state backed by an event log.
10. **Rich notifications with depth addresses** — notifications can return the user to the depth/pane that emitted them.
11. **Capability manifest + enforcement pass** — nested instances receive allowlisted filesystem, secret, network, hardware, spawn, pipe, and TTL capabilities.
12. **Secret broker MVP** — root mediates secret access for nested instances according to capability manifests.
13. **Typed pipes Phase 1** — manifest `[app.io]`, auto-wire, and linking UI for app composition.
14. **Plexi IQ Stage 1, depth-aware** — root/depth-aware orchestrator using Runs, `OpenIntent`, event bus, and capability checks.
15. **Portals + direct pipe promotion proof** — cross-depth view panes and focused-depth I/O path, at proof-of-concept quality.
16. **`[app.skill]` + `[app.agent]` manifest sections** — apps can expose invokable skills and installable agents.
17. **SDK 0.4.0 migration** — Python and Rust SDKs expose v2 fields/events and all bundled examples declare protocol v2.

### V2 Product

These user-facing surfaces land with or shortly after the protocol work:

- **App registry + `plexi install`** — remote registry, global installs, and project-scoped `--local` installs (#233).
- **App Store app** — catalog, developer publishing flow, one-click install.
- **Plexi Intelligence product surface** — BYOK first; managed keys can use the same secret injection path later.
- **`@agent` syntax** — resolves installed `[app.agent]` apps and prefers an already-running instance.
- **Fractal depth tree app/pane** — the validation app for recursive `.plexi` navigation.

---

## Explicitly Deferred — Not in 2.0

- **Trust/risk float learning** — v2 logs permission decisions; learning from them is later.
- **Agent replay testing** — v2's event bus makes this possible, but the replay engine is later.
- **WASM/PWA deployment** — back-burner until multiplayer or remote access requires it.
- **SpacetimeDB sync** — shared workspaces across machines.
- **Chat primitive** — typed pipes remain the composition primitive.
- **Core text editor primitive** — useful, but not required for recursive infrastructure.
- **Advanced UI SDK egui widgets** — v2 can ship without the broader widget layer.
- **Spatial canvas Option B/C beyond fractal depth** — v2 ships the `.plexi` depth tree; broader infinite canvas work waits.
- **Notification undo** (#223).
- **Production-grade hibernation** — `Suspend` is in scope; serializing and unloading deep inactive instances is not.
- **Full 3D depth visualization** — community/plugin work after the 2D/tree proof exists.

---

## Non-Goals — Will Never Be in Protocol v2

1. **No shared memory between instances.** PGAP is the isolation boundary.
2. **No inherited authority.** A child can attenuate capabilities for grandchildren, never amplify them.
3. **No arbitrary app-to-app RPC.** Host-mediated API calls and typed pipes remain the sanctioned paths.
4. **No invisible recursion.** If a directory is a Plexi instance boundary, the user must be able to see and reason about it.

---

## Release Checklist

Each item is discrete and verifiable. Flip the status box only when the "Done when" clause is true.

### Month 1 — Recursive Foundation

- [ ] **M1.1 — Protocol version negotiation** — `Init` carries `protocol_version`; manifests declare supported versions. **Done when:** a v1 app and v2 app both launch under a v2 host with correct negotiated behavior.
- [ ] **M1.2 — Process lifecycle foundation** — process groups, graceful shutdown, forced cleanup, `Suspend`, `Resume`. **Done when:** closing a pane reliably reaps an app process tree and older apps ignore lifecycle events safely.
- [ ] **M1.3 — Depth tree proof of concept** — discover `.plexi` directories and render them as depth nodes. **Done when:** the fixture in the Fractal roadmap displays nested agents/services and preserves parent layout while focusing a child.
- [ ] **M1.4 — Host event bus** — append-only JSONL events for app lifecycle and depth transitions. **Done when:** `tail -f ~/.plexi-alpha/events.jsonl` shows coherent app/depth events during a session.

### Month 2 — Recursive Protocol Surface

- [ ] **M2.1 — `OpenIntent` with depth context** — launches include caller, file/prompt/run, and optional depth address. **Done when:** launching a child depth/app does not require argv conventions.
- [ ] **M2.2 — Render summary protocol** — `RenderMode`, `StatusSummary`, `PaneSummary`, `Health`. **Done when:** a parent can request cheap child status without replacing the full visual frame.
- [ ] **M2.3 — TreeStatus rollup + depth notifications** — depth status and notifications bubble to root. **Done when:** root has a flat registry of active depths and clicking a notification can target its source depth.
- [ ] **M2.4 — Run primitive** — depth-scoped Runs backed by events. **Done when:** an agent task can move running → blocked-on-user → resumed → complete with depth context preserved.

### Month 3 — Isolation, Composition, Intelligence

- [ ] **M3.1 — Embedded Plexi spike** — `plexi --embedded` has a proven PGAP input/output path or a documented blocker. **Done when:** a script can send `Init`/`Render` JSON and receive valid PGAP JSON from an embedded process.
- [ ] **M3.2 — Capability manifest + secret broker MVP** — nested instances receive attenuated allowlists. **Done when:** a child can read an allowed path/secret and is denied outside its manifest.
- [ ] **M3.3 — Typed pipes Phase 1** — manifest `[app.io]`, auto-wire, linking UI. **Done when:** two unrelated apps compose via matching kind+name with no code changes.
- [ ] **M3.4 — Plexi IQ Stage 1, depth-aware** — IQ uses Runs, event bus, `OpenIntent`, and capability checks from root/depth context. **Done when:** agent mode delegates to an installed agent app and tracks the work as a depth-scoped Run.
- [ ] **M3.5 — Portals + direct pipe promotion proof** — portal pane and focused-depth I/O path exist at POC quality. **Done when:** a root pane can show a child depth and focused-depth interaction bypasses unnecessary intermediate render loops.
- [ ] **M3.6 — SDK 0.4.0 + migration pass** — bundled examples declare protocol v2 and SDKs expose v2 helpers. **Done when:** `just install-alpha` installs a clean v2 host with all bundled apps running.

### Product / Cross-cutting

- [ ] **P.1 — App registry + `plexi install`** — remote registry and project-scoped installs (#233).
- [ ] **P.2 — App Store app** — catalog, developer publishing flow, one-click install.
- [ ] **P.3 — Plexi Intelligence product surface** — BYOK through secrets CLI, managed keys later.
- [ ] **P.4 — `@agent` syntax in agent mode** — finds running agent instance first, otherwise spawns.
- [ ] **P.5 — Fractal depth tree app/pane** — user-facing validation surface for `.plexi` recursion.
- [ ] **P.6 — Secrets CLI** — Keychain-backed global/directory scoped secrets (#247).

### Hardening / Ops

- [ ] **O.1 — Alpha bug burn-down** — every issue labeled `alpha-bug` resolved before RC.
- [ ] **O.2 — RC pipeline** — `alpha` → `v2` branch → `v2.0.0-rc.N` tag via `just bump` → promote to `2.0.0` via `just release`.
- [ ] **O.3 — DEV_LOG + CHANGELOG complete** — every M/P/O item has a DEV_LOG entry on land and `CHANGELOG.md` has a user-facing `2.0.0` section.

---

## What Validates 2.0

- **Depth tree:** fixture `.plexi` directories render as navigable depth nodes.
- **Isolation:** nested instances cannot access paths/secrets/capabilities outside their manifest.
- **PGAP recursion:** embedded Plexi can be started as a subprocess and exchange PGAP JSON.
- **Lifecycle:** closing or crashing a child depth does not orphan descendants.
- **Event bus:** depth transitions, app spawn/close, notifications, pipe writes, permissions, and runs are visible in the event log.
- **Notifications:** a notification emitted from a child depth can navigate back to that depth/pane.
- **Typed pipes:** two unrelated apps compose via matching type/name declarations.
- **IQ:** agent mode delegates to an installed agent app and tracks the work as a depth-scoped Run.
- **Install:** the active branch can be installed with the correct build command and manually exercised in the app.

---

## Change Process — Editing This Document

1. **`CODEOWNERS` enforces review.** Any PR touching this file requires owner review.
2. **`Last updated` bump is required.**
3. **Paired edit rule.** Scope changes require a simultaneous edit to `plexi-v2.0.md` or a referenced sub-spec/roadmap.
4. **DEV_LOG entry required.** Every non-checklist edit needs a `[DECISION]` entry.
5. **Checklist status flips are exempt from rules 3 and 4**, but flipping an item to done requires a DEV_LOG entry documenting validation.
6. **Do not silently delete deferred items.** Move or reclassify them with an explicit note.

---

## References — The Full v2 File Set

1. `docs/VISION.md`
2. `docs/specs/README.md`
3. `docs/specs/releases/plexi-v2.0-scope.md`
4. `docs/specs/releases/plexi-v2.0.md`
5. `docs/specs/subsystems/fractal-pgap.md`
6. `docs/specs/roadmaps/fractal-pgap/`
7. `docs/specs/subsystems/app-infrastructure.md`
8. `docs/specs/subsystems/typed-pipes.md`
9. `docs/specs/subsystems/agent-orchestration.md`
10. `docs/specs/subsystems/agent-mode.md`
11. `schemas/plexi-manifest-schema.json`
12. `ROADMAP.md`
13. `DEV_LOG.md`

**Implementation entry points:** `src/app_protocol.rs`, `src/process_app.rs`, `src/pane_ops.rs`, `src/app_registry.rs`, `src/app_permissions.rs`, `src/app_api.rs`, `src/notification_log.rs`, `src/notify_socket.rs`, `src/context.rs`, `src/plexi_iq/`.

---

**End of scope spec.** Plexi 2.0 ships when recursion is visible, protocol-native, capability-scoped, and installable.

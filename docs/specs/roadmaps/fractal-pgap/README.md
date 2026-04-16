# Fractal PGAP Roadmap

**Status:** Roadmap for Plexi v2.0 scope
**Target:** Plexi v2.0
**Umbrella issue:** #260
**Subsystem:** [`../../subsystems/fractal-pgap.md`](../../subsystems/fractal-pgap.md)
**Preferred implementation worktree:** `fractal`

---

## Goal

Build an installable proof of concept that makes `.plexi` subdirectories visible as a recursive depth tree, then incrementally turns each depth into an isolated PGAP-speaking Plexi instance. This is the foundation of v2.0, not a post-v2 feature track.

The first demo does **not** need embedded rendering. It must prove the user-facing model:

- `.plexi` directories are discoverable instance boundaries.
- The current workspace can visualize the nested directory tree.
- The user can focus a depth and return to the parent without losing parent state.
- The code path is compatible with later `--embedded` instances, capability manifests, portals, and direct pipe promotion.

---

## Relationship to Existing v2 Primitives

Fractal PGAP is the v2.0 frame. Existing v2 primitives stay in scope because they are the mechanisms that make recursive instances usable and safe.

Reusable v2.0 work:

- Protocol version negotiation gives nested instances a compatibility handshake.
- Event bus gives depth transitions and tree updates a durable event stream.
- `OpenIntent` gives child depth launches structured context.
- Runs give agents a depth-scoped task container.
- Rich notifications can later carry depth addresses.
- Capability enforcement gives nested instances a security foundation.
- Typed pipes give cross-pane data flow without inventing app-to-app RPC.

Fallback rule: if a v2 primitive has not landed when the visual POC starts, the fractal POC should use a narrow internal adapter and leave a TODO pointing at the expected primitive. Do not block the visual depth-tree POC on the full orchestration layer.

---

## Worktree Strategy

Use an isolated worktree named `fractal` for the proof of concept.

Recommended branch:

```text
feature/260-fractal-pgap-poc
```

Base from `alpha` unless the explicit goal is a beta-only demo. Project policy says v2 work lands on `alpha` first. If a beta-installable artifact is needed, promote the working branch to `beta` after the POC passes instead of starting from stale beta code.

After code changes, install the active build before reporting completion:

- `alpha` branch: `just install-alpha`
- `main` branch: `just install`
- `beta` branch or beta demo branch: `just install-beta`

---

## Execution Order

Each spec is intended to be handled by one Codex agent or one focused human pass. Do them in order unless a spec explicitly says it can run in parallel.

| Order | Spec | Outcome |
|---|---|---|
| 1 | [`01-process-lifecycle.md`](01-process-lifecycle.md) | Child processes are easier to reap and lifecycle events are protocol-safe. |
| 2 | [`02-depth-tree-poc.md`](02-depth-tree-poc.md) | Plexi visualizes `.plexi` subdirectories as recursive depth nodes. |
| 3 | [`03-render-summary-protocol.md`](03-render-summary-protocol.md) | Children can report lightweight status and preview data. |
| 4 | [`04-embedded-instance-spike.md`](04-embedded-instance-spike.md) | `plexi --embedded` has a proven rendering/input path or a documented blocker. |
| 5 | [`05-capability-containers.md`](05-capability-containers.md) | Nested instances receive attenuated capabilities and cannot amplify them. |
| 6 | [`06-portals-and-direct-pipes.md`](06-portals-and-direct-pipes.md) | Cross-depth views and focused-depth I/O avoid unnecessary intermediate work. |

---

## End-to-End Fixture

Create a small fixture workspace for manual and automated verification:

```text
tmp/fractal-fixture/
  .plexi/
    config.toml
    canvas.toml
  agents/
    scraper/
      .plexi/
        config.toml
        canvas.toml
    reviewer/
      .plexi/
        config.toml
        canvas.toml
  services/
    api/
      .plexi/
        config.toml
        canvas.toml
```

The fixture is intentionally directory-first. It proves the model before embedded Plexi instances exist.

---

## Acceptance Criteria

- The spec index links to this roadmap and to the umbrella proposal.
- The roadmap can be followed by agents in series without re-reading the entire issue.
- The first implementation worktree is named `fractal`.
- The first POC can be verified without remote services or credentials.
- Every implementation spec names source files, tests, manual checks, and dependencies.

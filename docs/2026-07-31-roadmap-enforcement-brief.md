# ROADMAP.toml — convention and enforcement

Status: proposal awaiting ruling
Stint: none yet

## What the convention is

`ROADMAP.toml` at the repo root is **authoritative for current state, CI-enforced**. It is not a plan and not a wish list. It is the machine-checkable answer to "what actually works on alpha right now?"

One node per feature or concept:

```toml
[[node]]
id       = "capability-broker"
title    = "Capability broker — manifest declaration, scoped consent, precedence"
status   = "green"          # green | yellow | red
deps     = ["app-protocol"] # node ids
evidence = ["deny_beats_ask_beats_allow", "workspace_scoping_respected"]
stints   = []               # .stint task ids owning open work
```

- **green** — working and proven. **Requires named passing tests.** No evidence, no green.
- **yellow** — partially working, or working with a known defect / unproven surface. Evidence optional.
- **red** — not built, or built and known broken. Evidence is never treated as proof.

Evidence names are Rust `#[test]` function names (`cargo test --bin plexi`, plus `crates/*` test binaries) or TOML scene basenames under `tests/scenes/`. This is a repo-native convention, not a `stint` CLI feature.

It does not duplicate anything: `NORTH_STAR.md` is direction, PRMs are destination specs, `.stint` is work state. ROADMAP.toml is the only file that answers *state*.

## How CI enforces it

A `roadmap` job on every PR to alpha, next to `rust-host`:

1. **Parse and shape.** Valid TOML, unique ids, `status` in the enum, every `deps` id resolves, no dependency cycles.
2. **Green-requires-passing-named-tests.** For each green node, every `evidence` entry must (a) resolve to a real test function or scene file in the tree, and (b) be in the passing set of the same run's `cargo test` output. An unresolvable name, a name that does not run, or a name that fails → merge blocked. Names in CI's documented skip list do not count as passing.
3. **Evidence hygiene on non-green.** Warn (not block) when a red node lists evidence.
4. **Stint reference check.** `stints` ids must exist. `.stint/` is gitignored, so this runs advisory-only in CI and hard in the local `just roadmap-check`.

A prototype validator already exercises steps 1–2 against this draft: 63 nodes, zero unresolved evidence names.

The pragmatic sequencing: land the file, then land the checker, then make it required. Not one PR.

## The graph render (future work, not built here)

On merge to alpha, a job renders `deps` to a Mermaid graph colored by status and commits it to the website docs. Mermaid because it needs no toolchain and renders inline on GitHub. The render is generated output — never hand-edited, never a second source of truth. Not in scope for this PR.

## Audit findings worth your attention

- **63 nodes: 41 green, 8 yellow, 14 red.** The green set is genuinely well covered — 2,213 test functions on alpha and the CI gate landed in #2529.
- **The honest yellows**: WASM runtime (a guest dying at import is only caught by a 15s timeout, and that test is explicitly skipped in CI — 0638/0663); frame pacing (Python and WASM paths still not unified, 0553 blocked); media-io (mock round-trips and no-panic smoke, no real decode proof); secrets (`src/secrets` has zero unit tests of its own); the CI gate itself (one documented skip, one undeleted bundle-strategy branch).
- **The reds that are pure spec**: browser surface (eleven stints, all blocked on an unstarted spike), marketplace + monetization, video editor, agent run orchestration, app state scopes, MCP tool sources.

## Open questions for you

1. **Granularity.** 63 nodes is one per meaningful surface. Too fine? The natural coarser cut is ~25 (fold spatial-navigation into tiling, notes into text-editor, etc.).
2. **Blocking vs advisory to start.** Recommend advisory for one week so the first wave of "this green is actually yellow" corrections does not block merges.
3. **Who updates it.** Recommend: the same PR that changes a node's state updates the node, enforced by review, not by a bot.
4. **Scenes as evidence.** I counted TOML scenes as valid evidence names. Confirm — or restrict green to Rust `#[test]` only.
5. **`stints` field with a gitignored `.stint/`.** CI cannot verify those ids. Accept advisory-only, or drop the field?

RECOMMENDATION:
1. Ship the file as drafted at 63 nodes, run the checker advisory for one week, then flip it to required.

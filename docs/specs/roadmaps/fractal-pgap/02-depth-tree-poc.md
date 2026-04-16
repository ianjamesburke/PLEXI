# 02 — Depth Tree Proof Of Concept

**Goal:** Visualize `.plexi` subdirectories as recursive depth nodes before embedded rendering exists.

---

## Scope

- Discover `.plexi` directories beneath the current workspace root.
- Represent each `.plexi` directory as a depth node with path, display name, child count, and last scan time.
- Add a Rust-native pane/app that renders the depth tree.
- Support focusing a node and returning to the parent in UI state.
- Persist enough state to restore the focused depth after restart.

---

## Non-Goals

- No `plexi --embedded` yet.
- No direct pipe promotion.
- No capability manifests.
- No cross-depth portals.
- No background agent spawning.

---

## Relevant Files

- `src/app_protocol.rs`
- `src/app_registry.rs`
- `src/pane_ops.rs`
- `src/context.rs`
- `src/file_browser/mod.rs`
- `docs/specs/subsystems/fractal-pgap.md`

The exact app/module name can be chosen during implementation. Prefer a small built-in app if it needs direct access to workspace state; prefer an external app only if the needed tree data can be supplied through PGAP cleanly.

---

## UX Contract

- A `.plexi` directory marks an instance boundary.
- The depth tree is derived from the filesystem, not from manually maintained metadata.
- The first visual version can be simple: rows, indentation, status text, and selected node highlight.
- It must be obvious which node is the current depth.
- Selecting a depth must not destroy the parent pane layout.

---

## Fixture

Use the fixture described in [`README.md`](README.md). Tests should create it under a temporary directory rather than depending on user files.

---

## Tests

- Discovery test: nested fixture returns the expected nodes and parent/child relationships.
- Ignore test: directories without `.plexi` are not depth nodes.
- State test: selected depth serializes and restores.
- UI smoke test: app/pane renders at least one node for the fixture.

---

## Manual Verification

1. Create the fixture workspace.
2. Open Plexi at the fixture root.
3. Open the depth tree pane.
4. Confirm `agents/scraper`, `agents/reviewer`, and `services/api` appear.
5. Focus `agents/scraper`, return to root, and confirm root layout is still intact.

---

## Done When

- A user can see and navigate the recursive `.plexi` directory model.
- The proof of concept can be installed and tested locally without embedded instances.
- The implementation leaves clean extension points for render summaries and embedded children.

---
id: "0184"
title: "CLI-native PGAP apps: Board/Console primitives + stint board"
status: backlog
estimate: "40h"
sprint: "s32"
blocked_by: []
gh_issue: []
area:
  - "cli/commands"
  - "sdk/pgap"
  - "host/ui"
  - "host/terminal"
tags:
  - "v1-late"
  - "app-authoring"
  - "cli-renderer"
  - "host-ui-kit"
---

## Why

Let an arbitrary installed CLI ship a bespoke Plexi app (not the generic auto-renderer)
that `plexi app open <cli>` launches. Proven end-to-end by a keyboard-driven,
drag-and-drop **stint sprint board** bundled in the stint repo.

Full design + grounded current-state audit + milestones + open decisions:
**`docs/prm/cli-native-pgap-apps.md`** (the canonical planning source for this work).

Key finding: the spawn mechanism is ~80% built. The `--plexi` descriptor already has a
`plexi_app` field (`src/app/plexi_descriptor.rs:44`) and the host already spawns it as a
full custom PGAP app, terminal-free (`src/pane_ops/create.rs:981`). The new work is a
routing fix + two UI primitives + a capability split + the stint-side app and commands.

## Scope

Cross-repo. See the PRM for per-milestone detail (M1-M6).

PLEXI:
- M1 — `--cli` and bare-id open paths converge and honor `plexi_app` (routing fix).
- M2 — new `process.run` capability + `emit.run()` ergonomics + `Console` (SDK composite
  over ScrollLog for v1). Terminal-free command execution into an in-app console.
- M3 — `Board` L1 primitive (host-rendered columns/cards, keyboard nav + drag, one
  `move {card, from, to}` event) + Python `Board/BoardColumn/BoardCard` wrappers.
- M6 — auto-renderer adopts in-app Console; linked terminal demoted to opt-in.

stint (`~/Documents/GitHub/stint`):
- M4 — `stint --plexi` descriptor + embed/extract (include_dir!); `stint move <id> --sprint <s>`;
  `stint list --json`.
- M5 — the bundled Python board app (board + search + area filter + console).

Decision #1 (load-bearing): app language. Recommend Option A (Python, embedded+extracted,
reuses existing SDK + github-issues reference) for v1; Rust PGAP SDK is a separate
north-star epic for true single-binary distribution.

## Gotchas

- This is one task tracking a multi-milestone PRM; split into per-milestone tasks
  (and GitHub issues) when promoted out of backlog. M1+M3 are the critical path; M4 can
  run in parallel in the stint repo.
- Data contract: the app reads via `stint list --json` / writes via `stint move` only —
  never parse/rewrite `.stint/*.md` directly (couples to format, bypasses `stint check`).

## References

- `docs/prm/cli-native-pgap-apps.md` — full PRM (design, milestones, file index, open decisions)
- `src/app/plexi_descriptor.rs:27-53`, `src/pane_ops/create.rs:981-1136` — existing plexi_app spawn
- `src/cli/open.rs:230-283` — routing fix target
- `apps/github-issues/main.py` — reference app to mirror
- `docs/prm/host-ui-kit.md` — Board belongs in the shared host UI kit
- `~/Documents/GitHub/stint/crates/stint-cli/src/main.rs`, `crates/stint-core/src/serialize.rs`

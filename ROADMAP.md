# Plexi Roadmap

This file is an index, not the dispatch queue.

The canonical plan for app authoring, packaging, marketplace trust, MCPUI, WASM/WASI, and Bevy is [`docs/prm/app-framework-marketplace.md`](docs/prm/app-framework-marketplace.md). If this file and the PRM disagree, the PRM wins.

## Current Focus

Work is moving through the marketplace PRM in this order:

1. Finish app authoring: generated apps use `view()` and L1 components, Core apps serve as references, `TextEdit` works as a normal tree child, and app-authoring tests cover render/input/state/layout.
2. Clean up permissions and trust: app powers go through host-mediated APIs, manifests declare real powers, and Python apps are labeled as reviewed native processes.
3. Define packages and local install: package metadata, validation, checksums, runtime requirements, capabilities, trust labels, and local install before hosted marketplace work.
4. Add hosted marketplace: registry, publisher accounts, submission review, paid apps, revenue share, refunds, takedowns, analytics, and Plexi AI subscription as an `ai.query` backend.
5. Add runtime lanes: MCPUI export/import, WASM/WASI sandboxing, `Surface`, and Bevy targeting WASM + `Surface`.

Use the PRM for the actual Done When checks and test plan.

## Already Stabilized

The old stabilization and polish layers are no longer the planning surface. Use `git log`, closed issues, and `GOTCHAS.md` when you need history.

Recent completed areas include the declarative keybinding table, shared modal shell, CLI namespace cleanup, pane spawning unification, app viewport overtake, terminal search, notification polish, text editor extraction, Core app theming, SDK v2 scaffolding, and PGAP reference work.

## Parallel Work

Pane lifecycle work remains valid product work, but it is not part of the marketplace PRM:

- pane-level hiding
- context-level parking
- inventory overlay
- notifications from hidden panes
- restore-to-layout
- context operations
- live miniature rendering for portal tiles
- pane activity indicators

Do not let this track displace the marketplace path unless the user explicitly pivots.

## Dispatch Rules

Do not use GitHub Project board #7, `NEXT.md`, `/pick-parallel`, `/update-next`, or stale blitz state.

To plan dispatch:

1. Read `docs/prm/app-framework-marketplace.md`.
2. Identify the first unfinished milestone.
3. List open GitHub issues that match that milestone.
4. Skip blocked and in-progress issues.
5. Pick parallel lanes with non-overlapping `area:*` labels.
6. If the PRM calls for work that has no issue, create or triage the issue first.

GitHub issues are work tickets. GitHub milestones are optional release buckets, not the planning source.

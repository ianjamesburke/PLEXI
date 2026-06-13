---
id: "0012"
title: "App authoring: verification harness and docs"
status: todo
estimate: "12h"
sprint: "s2"
blocked_by:
  - 9
gh_issue: []
area:
  - "infra/docs"
  - "sdk/python"
  - "sdk/pgap"
tags:
  - "app-authoring"
  - "docs"
  - "verification"
---


Add acceptance coverage and docs proving generated apps render, handle input, save state, and avoid layout overlap.

## Why

The app authoring milestone is complete only when agents can verify the app they generated without relying on visual guesswork.

## Scope

- Add TOML scene tests (`tests/scenes/`) covering: scaffold app renders on first frame, FooterKeys visible at minimum pane size, TextInput accepts keystrokes, state persists across reload.
- Add `plexi app validate <path>` CLI command (or extend `plexi app health`) that runs the render-inspect loop from 0009 non-interactively.
- Document the full app authoring flow end-to-end in `docs/SDK_QUICKSTART.md`: init, dev, test, validate, publish.
- Include marketplace publish as the final step in the docs flow.

## Gotchas

- Tests define done. Avoid merging scaffold or docs changes that are not exercised by the render/inspect loop.
- Scene harness is `src/scenes.rs`, UI harness is `src/ui_tests.rs`. 0165 already shipped shortcut harness parity; build on those fixtures.
- `docs/sdk-v2.md` and `docs/SDK_QUICKSTART.md` both exist. Consolidate if they overlap; do not maintain two parallel authoring docs.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/sdk-v2.md`
- `docs/SDK_QUICKSTART.md`
- `src/scenes.rs`, `src/ui_tests.rs` (test infrastructure)
- `docs/TESTING.md` (test conventions)

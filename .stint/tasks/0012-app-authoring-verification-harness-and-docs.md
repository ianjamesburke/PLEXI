---
id: "0012"
title: "App authoring: verification harness and docs"
status: done
estimate: "12h"
actual: "6m"
started_at: "2026-06-13T08:34:47Z"
completed_at: "2026-06-13T08:40:29Z"
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

## Outcome

Much of the harness already exists: `plexi app check` is the non-interactive
render-inspect validator (manifest + SDK-shape + render-size matrix), and
`plexi app validate` (package check) is wired and unit-tested in
`src/cli/validate.rs`. So the new work was the end-to-end proof + docs:

- **`apps/dev/counter`** — a real `plexi app init` scaffold (manifest with the
  0008 `[marketplace]` placeholder, view()-based counter, justfile) committed as
  the scaffold-sample fixture.
- **`tests/scenes/scaffold-renders-frame1.toml`** — proves the scaffold output
  renders on first frame (AppBar "Counter") and survives a keystroke without
  crashing. Passes; shot captured.
- **`docs/SDK_QUICKSTART.md`** — new "Verify And Publish" section documenting the
  full init→dev→verify→publish flow (`check` / `validate` / `package` / `publish`),
  plus a note clarifying this Quickstart is the *tutorial* and `sdk-v2.md` is the
  *reference* (not two parallel authoring docs — kept distinct by role).

Existing scene coverage already satisfies most acceptance goals: render
(`scaffold-renders-frame1`, `footer-small-pane`, `assistant-idle`), input
(`keymap-probe` proves keys reach the app), min-pane FooterKeys
(`footer-small-pane`, 280px).

**Deferred (filed):** the state-persists-across-reload scene needs a scene-level
reload verb that doesn't exist (`Step` enum has no close/reload). Filed as
**#2243** rather than building a half verb here.

## Variance

Estimate 12h. The verification commands (`check`/`validate`) and most scene
coverage already existed, so this was the scaffold fixture + one new scene + the
end-to-end docs + filing the reload-verb gap — not building a harness from
scratch.

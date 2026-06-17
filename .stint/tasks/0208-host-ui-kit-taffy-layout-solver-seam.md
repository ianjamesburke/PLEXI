---
id: "0208"
title: "Host UI kit: taffy layout-solver seam (declare -> solve -> paint)"
status: backlog
estimate: "3h"
blocked_by: []
gh_issue: []
area:
  - "ui/widgets"
  - "infra/testing"
tags:
  - "v2"
  - "ui"
  - "testing"
  - "layout"
---

The host UI kit centralized modal/list/input primitives in `src/ui/`, but the primitives still hand-compute pixel positions internally. `ListRow` returns a `TextBlockMetrics { primary_center_y, secondary_center_y }` and each draw site picks a scalar to align against. On a single-line row the title center equals the row center, so it looks correct; add a subtitle and they diverge, which is how the metadata chip rode high on two-line Notes rows. This is the exact "no vertical-centering contract" defect `docs/prm/host-ui-kit.md` named as the proving case — centralization fixed *where* the math lives, not *that* there is hand-rolled math. This task installs a real layout-solver seam so the bug class becomes unrepresentable: callers declare a cell structure, a pure solver computes every rect against the row box with one alignment rule, and paint is a dumb consumer that never chooses a coordinate.

Use the renderer-agnostic **`taffy` crate (^0.9) directly** as the solve stage. Do NOT use `egui_taffy` — its 0.12 release requires egui ^0.34 (Plexi is on 0.31) and it is built to drive egui *widgets* through taffy, whereas we want taffy to return rects we paint ourselves (preserving z-order control, the danger-glow-behind-content paint order, and headless-harness rendering). `taffy` core has no egui dependency, so no coordinated egui bump is needed.

## Scope

- Spike first (step 0): prove `taffy` can solve a `ListRow`'s cells (leading slot, title, subtitle, trailing chip, metadata lane, pips, trailing action) into rects, and that painting those rects via `ui.painter()` reproduces current `ListRow` visuals. Abort/redesign if interop is poor before building the seam.
- Define a typed row structure (the "declare" layer): the set of cells a row can contain plus their alignment intent (e.g. row-centered vs title-baseline).
- Build the pure "solve" function: takes the row rect + cell structure, returns a concrete rect per cell via `taffy`, applying one vertical-centering rule against the row box. No draw site computes a center after this lands.
- Refactor `ListRow::show` to "paint": consume solved rects only. Delete `TextBlockMetrics` and the `primary_center_y`/`secondary_center_y` scalar threading.
- Pure unit tests on the solver (no rendering): assert e.g. `chip.center().y == rect.center().y` for one- and two-line rows, reserved-width and right-lane math.
- `egui_kittest` golden-image tests (already a dep, currently unused for rows) over the permutation matrix: 1-line, 2-line, +chip, +metadata chips, +pips, +trailing action, selected, dense.
- Remove the localized stopgap fix in `src/ui/list.rs::draw_metadata` (the two-line `rect.center().y` branch) once the solver supersedes it.

## Non-Scope

- Migrating the other ~17 painter-based sites (`sidebar_row.rs`, `button.rs`, `toast.rs`, `surface.rs`, `shortcuts.rs`, overlays, render pipeline) onto the seam — those are follow-on tasks once `ListRow` proves the pattern.
- Bumping egui 0.31 -> 0.34. Explicitly avoided by using raw `taffy`.
- The Plexi-owned docking/tile layout engine (see `0190`) — that is the cross-window/tile layer; this is within-widget cell layout. Both may share the `taffy` adoption; this task is the foundational beachhead, not a blocker for 0190.

## Why

The off-center chip is the second occurrence of a bug class the host UI kit was built to eliminate; without a solver enforcing the layout contract inside the primitives, it will keep recurring. This makes it structurally impossible and gives the kit CSS-flexbox-grade declarative layout without leaving the egui canvas or the headless test harness.

## References

- `src/ui/list.rs` — `ListRow`, `draw_text_block`, `draw_metadata`, `TextBlockMetrics`; the stopgap branch to remove
- `docs/prm/host-ui-kit.md` — "Purpose" names this bug as the proving case; "Non-Goals" keep egui as the renderer
- `src/ui_tests.rs` — `PlexiUiHarness` headless render harness for golden tests; `egui_kittest` already in `Cargo.toml`
- `NORTH_STAR.md` — "Pixel math in app code" exclusion; this applies the same declare-structure / host-solves-layout rule to host chrome
- `.stint/tasks/0190-*.md` — related v2 docking layout engine; shares taffy adoption, not blocked by this

# Live Edge Preview Terminals

## Purpose

This document describes a future enhancement for focus mode: add interior padding around the active terminal and show `20px` edge strips for directional neighbor terminals (`left`, `right`, `up`, `down`) when those neighbors exist.

This is a **future enhancement** document, not MVP scope by default.

## Summary

The feature is feasible with manageable CPU cost if rendering is capped and adaptive. CPU risk becomes high only when many preview terminals are rendered continuously without throttling.

Chosen defaults:

- Live edge strips enabled in adaptive mode
- Strip thickness fixed at `20px` per visible side
- Main terminal gets additional focus-mode padding
- Clicking a strip focuses that directional neighbor
- No new UI or config controls in v1

## Product Behavior

### Focus mode layout

- The active terminal remains the primary interactive surface.
- The active surface is inset by a fixed padding value.
- If a directional neighbor exists on a side, reserve a `20px` strip on that side for preview.
- If no neighbor exists on a side, no strip is shown on that side.

### Edge strip interaction

- Edge strips are clickable.
- Clicking a strip performs directional focus to that neighbor and re-renders focus mode.
- Keyboard directional focus remains unchanged.

### Live preview meaning

- "Live" means near-real-time output preview of neighbors.
- Edge strips are read-only preview surfaces, not fully interactive terminals.
- Active panel input/output behavior is unchanged and remains highest priority.

## Technical Direction

### Rendering model

- In focus mode, render one active terminal plus up to four preview surfaces (one per side).
- Neighbor selection comes from directional adjacency logic already used for focus navigation.
- Hard cap previews to `4` surfaces total.

### Runtime lifecycle

- Keep active runtime fully interactive and unthrottled.
- Preview runtimes are read-only and exist only for currently visible side neighbors.
- Destroy preview runtimes when focus mode exits, neighbors disappear, or panel focus changes.

### Adaptive performance mode

- Batch preview writes on animation frames, not per output chunk.
- Apply per-preview byte budgets per frame; overflow remains buffered.
- Pause preview updates for hidden or non-neighbor panels.
- Auto-downgrade a side from live to snapshot mode when sustained output or frame budget pressure is detected.
- Auto-recover to live mode after pressure drops and stability returns.

## Acceptance Criteria

1. Focus mode shows additional padding around the active terminal.
2. Exactly `20px` strips appear only on sides where directional neighbors exist.
3. Clicking a visible side strip focuses the expected neighboring panel.
4. Keyboard directional navigation behavior is preserved.
5. Under heavy adjacent output, active terminal responsiveness remains stable.
6. Adaptive downgrade and recovery behavior can be observed in automated verification.

## Test Plan

### Unit and state tests

- Directional neighbor mapping returns expected panel per side.
- Strip visibility toggles correctly as panels are added, moved, focused, or closed.
- Side click routing maps to the same neighbor used by directional focus.

### E2E tests

- Focus mode renders active padding.
- `20px` strip rendering matches directional neighbor presence.
- Clicking each side strip focuses the expected neighbor.
- Existing keyboard directional focus behavior remains intact.

### Performance checks

- Synthetic high-output adjacent panels do not degrade active panel interactivity.
- Adaptive downgrade triggers under pressure.
- Live preview recovery occurs once load normalizes.

## Rollback / Safety

If performance regression is observed:

1. Disable live preview updates and keep snapshot strips only.
2. Disable strips entirely while retaining active terminal padding.
3. Re-enable in phases after profiling confirms acceptable frame and input responsiveness.

## Non-Goals (for this enhancement)

- No user-facing settings UI for padding or strip size.
- No arbitrary number of live preview terminals.
- No changes to PTY backend protocol.
- No behavior changes in overview mode.

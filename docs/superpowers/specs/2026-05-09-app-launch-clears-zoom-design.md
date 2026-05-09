# App Launch Clears Zoom

**Date:** 2026-05-09  
**Status:** Approved

## Problem

Launching an app while a pane is zoomed (`Cmd+Enter`) silently places the new app behind the zoom overlay. The user sees nothing. Splits already clear zoom before executing; app launch does not. This is an inconsistency that produces invisible panes.

## Decision

Zoom is a view state, not a mode. Launching an app is an intentional navigation act that supersedes whatever the user was looking at. App launch clears `zoomed_pane` before proceeding.

The `overlay` hint is exempt — overlay replaces the focused pane in-place and is designed to work at any zoom state (e.g. secrets overlay, file browser).

## Change Surface

Two sites in `src/pane_ops/create.rs`, both exempt the `overlay` hint:

- `open_process_app_pane`: clear `self.windows[active].zoomed_pane` before the split path
- `open_builtin_app_pane`: same guard

The guard belongs at these lowest-level entry points so no future caller can bypass it.

## Behavior

| Trigger | Was zoomed? | Result |
|---|---|---|
| Open app from palette (split hint) | Yes | Zoom clears, app splits normally |
| Open app from palette (no hint) | Yes | Zoom clears, app splits normally |
| Open overlay app (secrets, file browser) | Yes | Zoom stays; overlay replaces the zoomed pane |
| Open app while not zoomed | No | No change |

## Logging

`log::info!("app::{id}: cleared zoom before launch")` when the guard fires.

## Rejected Alternatives

**Overlay-on-zoomed-pane stack:** New app replaces the zoomed pane as overlay; Esc pops back. Rejected — `overlay_replaced` chaining has no back affordance in current UX. Creates unbounded stack depth with no clear navigation model.

**Dedicated app layer:** Apps live in a separate full-screen layer above tiling. Rejected — premature architectural change; breaks existing app-in-split behavior; only warranted if "apps are always full-screen" becomes a firm product direction.

**Confirmation dialog ("Replace current instance?"):** Rejected — conflates view state (zoom) with process identity; adds friction for a non-destructive action; the user explicitly triggered "open app."

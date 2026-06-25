---
skill_version: "0.0.440"
description: SDK guidance for building Plexi Python apps — conventions, gotchas, and patterns to follow when writing or reviewing SDK app code.
---

# Plexi SDK — Agent Guidance

This file is a stub for accumulating SDK-level guidance. Expand it when new
patterns are established or when a recurring mistake is worth preventing.

## Key Conventions

- Key canonical names are **lowercase**: `"return"`, `"escape"`, `"backspace"`. Never match `"Enter"` or `"Escape"` — the SDK normalizes those from egui's internal names.
- Enter = open/confirm. Escape (+ optional Backspace) = exit/cancel. Every focused sub-view must be escapable.
- See `README.md` for the full keyboard conventions table.
- SDK v3 apps are module-level `init(size, args)`, `update(event)`, and `view()` functions. Do not subclass `App`, define `on_render`, or call `.run()`.

## Widget Selection

- For list+detail navigation: use `SelectList` from `plexi_sdk.ui` — it handles j/k/arrow keys, scrolling, and click hit-testing. Never reimplement this by hand.
- For raw drawing or games: return a `Canvas(...)` from `view()` and update runtime state from `RenderFrame` events.
- For text entry: use `ui.TextEdit` in the component tree. Never read raw keys for text.

## State

- Keep state in `plexi_sdk.state` and update it by returning `SetState` / `state.set(...)` effects from `update(event)`. `view()` must stay pure.

## Logging

- Always log at init and key state transitions with `plexi_sdk.log`.
- Log escapes and errors from `update(event)`: `log.warn("app_id: exiting detail view")` on Escape, `log.error(...)` on unrecoverable failures.

## Known Gotchas

- `plexi_sdk` is only importable from processes spawned by Plexi. Test by opening the app in a pane, not by running `python3 -c "import plexi_sdk"` in a terminal.
- SDK proxy wrappers are not auto-generated — check `plexi_sdk/` source for what actually exists before writing code that calls undocumented methods.

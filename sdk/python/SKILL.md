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

## Widget Selection

- For list+detail navigation: use `SelectList` from `plexi_sdk.ui` — it handles j/k/arrow keys, scrolling, and click hit-testing. Never reimplement this by hand.
- For raw draw-only list: `ctx.list_view()` with `ListItem` dicts.
- For text entry: `ctx.text_input()` or `widgets.TextInput`. Never read raw keys for text — use `on_text_submitted`.

## State

- Stateful widgets (`SelectList`, `ListView`, `TextArea`, `Keymap`) must be created in `on_init`, not `on_render`. Creating them per frame resets state every render.

## Logging

- Always log at init and key state transitions: `ctx.info(...)` inside a frame, `emit.info(...)` outside.
- Log escapes and errors: `ctx.warn("app_id: exiting detail view")` on Escape, `ctx.error(...)` on unrecoverable failures.

## Known Gotchas

- `plexi_sdk` is only importable from processes spawned by Plexi. Test by opening the app in a pane, not by running `python3 -c "import plexi_sdk"` in a terminal.
- SDK proxy wrappers are not auto-generated — check `plexi_sdk/` source for what actually exists before writing code that calls undocumented methods.

---
id: "0165"
title: "PGAP SDK shortcut helper and harness parity"
status: backlog
sprint: "s2"
estimate: 6h
blocked_by: []
gh_issue:
  - "2196"
area:
  - "sdk/python"
  - "sdk/pgap"
  - "infra/testing"
tags:
  - "app-authoring"
  - "sdk-v2"
  - "shortcuts"
  - "testing"
---

Add a first-class Python SDK shortcut helper so PGAP apps can bind command shortcuts without handwritten raw key/modifier branching, and make the scene harness mirror live printable-key delivery.

## Why

App authors should not need to know whether a printable shortcut originated as an egui `Event::Key` or `Event::Text`. The SDK should expose the obvious command-shortcut path, while text entry remains owned by TextInput and text hooks.

## Scope

- Extend or replace the existing SDK `KeyMap` helper with a shortcut API for bare printable keys, named keys, and modifier chords.
- Preserve the existing PGAP `on_key(key, mods)` contract; this is a normalization helper, not a second protocol event model.
- Add real PGAP scene coverage proving `key = "z"` reaches the helper the same way live Plexi does.
- Keep app guidance clear: shortcut helper for commands, TextInput/text hooks for typed text.

## Gotchas

- The host intentionally forwards printable keys through text-backed delivery to avoid double-dispatch and preserve OS-resolved characters.
- Scene key injection must stay in parity with live app input, or smoke tests will accept paths that users cannot exercise.
- ListView/component-owned navigation can intercept some bare keys; docs and tests should avoid implying every key is globally available in every component state.

## References

- GitHub issue #2196
- `sdk/python/plexi_sdk/widgets/keymap.py`
- `sdk/python/plexi_sdk/_app.py`
- `src/process_app/mod.rs`
- `src/ui_tests.rs`
- `src/scenes.rs`

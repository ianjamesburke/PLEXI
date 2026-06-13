---
id: "0159"
title: "UI hardening: multiline host TextField"
status: todo
estimate: "12h"
sprint: "s31"
blocked_by: []
gh_issue:
  - "2173"
area:
  - "ui/widgets"
  - "ui/overlays"
tags:
  - "ui-hardening"
  - "host-ui-kit"
  - "text-field"
  - "focus"
---


Add a multiline host TextField primitive and migrate text-owning overlays that still hand-style multiline `TextEdit`.

## Why

Single-line host inputs are centralized, but QuickNote compose, notification prompts, and edit-description still own raw multiline styling. The host kit needs a multiline path that preserves modal focus behavior.

## Gotchas

- `GOTCHAS.md` documents the two-layer egui TextEdit focus problem; preserve both one-shot and post-CentralPanel focus behavior.
- QuickNote paste handling is fragile; preserve paste queue behavior and Enter/Shift+Enter semantics.
- Do not force document editing (`TextEditorApp`) into a modal text field unless the behavior really matches.

## References

- GitHub issue #2173
- `src/ui/text_field.rs`
- `src/overlays/quick_note.rs`
- `src/overlays/notification_modal.rs`
- `src/overlays/misc.rs`
- `GOTCHAS.md`

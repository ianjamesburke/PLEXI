---
id: "0160"
title: "UI hardening: notification modal footer and action chrome"
status: done
estimate: "12h"
actual: "0m"
started_at: "2026-06-15T07:48:49Z"
completed_at: "2026-06-15T07:48:49Z"
blocked_by: []
gh_issue:
  - "2174"
area:
  - "host/notifications"
  - "ui/overlays"
  - "ui/widgets"
tags:
  - "ui-hardening"
  - "host-ui-kit"
  - "notifications"
  - "modal"
---



Move notification modal footer and action-row chrome onto shared host UI primitives.

## Why

The notification modal now uses `ModalShell`, but it still has the densest custom footer and action-row layout in host chrome. Shared row, surface, shortcut, and hint primitives should own more of that paint and spacing.

## Gotchas

- Preserve keyboard direct-select behavior and reserved shortcut stripping.
- Preserve prompt input focus and required/optional submit behavior.
- Extend `HintBar` only if the wrapping/multi-row behavior is genuinely reusable.

## References

- GitHub issue #2174
- `src/overlays/notification_modal.rs`
- `src/ui/hints.rs`
- `src/ui/row.rs`
- `src/ui/surface.rs`

## Reconciliation

Marked done in this branch after auditing current alpha. The implementation had already landed: notification prompts use `TextArea::multiline`, action rows use shared `choice_button`, and footer shortcuts use `HintBar`.

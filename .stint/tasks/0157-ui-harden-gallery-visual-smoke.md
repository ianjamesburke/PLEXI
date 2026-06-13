---
id: "0157"
title: "UI hardening: host UI gallery visual smoke coverage"
status: todo
estimate: "8h"
sprint: "s31"
blocked_by: []
gh_issue:
  - "2175"
area:
  - "ui/widgets"
  - "ui/overlays"
  - "infra/testing"
tags:
  - "ui-hardening"
  - "host-ui-kit"
  - "gallery"
  - "testing"
---


Add a lightweight visual smoke path for Host UI Gallery and key host overlays.

## Why

The host UI kit is now split and centralized, but build-only verification will not catch visual drift. The gallery should become a reliable smoke surface for PR validation.

## Gotchas

- Prefer deterministic checks over pixel-perfect snapshots for the first pass.
- Keep the initial smoke path lightweight; do not build broad CI infrastructure unless the local check proves stable.

## References

- GitHub issue #2175
- `src/overlays/ui_gallery.rs`
- `src/ui_tests.rs`
- `src/testing/mod.rs`
- `.agents/skills/validate-pr/SKILL.md`

---
id: "0151"
title: "UI hardening: finish host chrome button primitive migration"
status: backlog
sprint: "s31"
estimate: 8h
gh_issue: ["2172"]
area: ["ui/widgets", "ui/sidebar", "ui/chrome", "cli/commands"]
tags: ["ui-hardening", "host-ui-kit", "buttons", "chrome"]
---

Finish moving remaining host chrome buttons onto focused `ui::button` primitives.

## Why

The themed egui refactor centralized modal action chrome, but a few toolbar/sidebar/CLI-renderer buttons still encode shape, color, and hover behavior inline. They should either use shared button primitives or force a deliberate new variant.

## Gotchas

- Keep app protocol renderer buttons out of scope unless they are host chrome.
- Do not hide custom behavior behind a too-generic button kind; add explicit variants only when a repeated host shape exists.

## References

- GitHub issue #2172
- `src/ui/button.rs`
- `src/ui/sidebar.rs`
- `src/overlays/toolbar.rs`
- `src/render/cli_renderer_app.rs`

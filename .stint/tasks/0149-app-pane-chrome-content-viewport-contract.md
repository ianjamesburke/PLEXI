---
id: "0149"
title: "App pane chrome: host-owned content viewport contract"
status: done
estimate: "16h"
actual: "20m"
started_at: "2026-06-13T22:08:31Z"
completed_at: "2026-06-13T22:27:47Z"
sprint: "s5"
blocked_by:
  - 23
gh_issue:
  - "2152"
area:
  - "ui/chrome"
  - "host/pane-ops"
  - "sdk/pgap"
tags:
  - "v1"
  - "ui"
  - "chrome"
  - "layout"
  - "app-pane"
---




Give every app pane a host-owned chrome shell so apps render only inside the post-chrome content viewport.

## Variance Note

Estimated 16h, actual 20m (~48x under). The egui cursor-advance math was already correct; the task reduced to a structural refactor (removing the ui.vertical() branch, extracting chrome_height() for testability) plus 4 unit tests. The 16h estimate reflected architectural uncertainty that didn't materialise.

## Why

Conditional app-pane chrome, such as return/overtake bars and nav bars, should never leak into app layout math. Apps should render inside an already-constrained viewport; the host owns chrome bands and the content rect.

## Gotchas

- This is not a File Explorer-specific padding fix.
- `allocate_new_ui` can paint without advancing the parent cursor; chrome that consumes space must allocate in the parent layout.
- L1 component sticky footers should pin to the app content viewport, not the full pane rect.
- Remove tactical app-specific chrome tolerance once the content viewport contract is enforced.

## References

- GitHub issue #2152
- `src/render/app_pane.rs`
- `src/app/app_trait.rs`
- `src/process_app/mod.rs`
- `src/render/components.rs`

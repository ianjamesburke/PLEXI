---
id: "0010"
title: "App authoring: component polish and small-pane fit"
status: done
estimate: "8h"
actual: "9m"
started_at: "2026-06-13T07:55:42Z"
completed_at: "2026-06-13T08:04:15Z"
sprint: "s2"
blocked_by: []
gh_issue:
  - "2111"
  - "2240"
area:
  - "sdk/python"
  - "ui/widgets"
tags:
  - "app-authoring"
  - "components"
  - "small-pane"
---




Tighten SDK component defaults so generated apps fit small panes without footer clipping, text overlap, or egui-looking shortcut rows.

## Why

Generated apps will inherit every rough SDK component decision. The app authoring sprint must make those defaults boring and solid. This task unblocks 0011 (Core 9 sweep).

## Scope

- Fix `FooterKeys` vertical/horizontal centering (issue #2111): `sdk/python/plexi_sdk/ui.py:FooterKeys` class and host-side rendering in `src/process_app/render.rs`.
- Audit all SDK components (`ListView`, `FooterKeys`, `AppBar`, `TextInput`, `Label`, `Column`, `Spacer`) for small-pane (< 300px wide) fit.
- 0166 (align enum, TextInput chrome, Canvas leaf) already shipped. Do not re-touch align or TextInput chrome.

## Gotchas

- Keep host layout responsible for spacing and fit; do not push pixel math back into apps.
- `key_combo_list` in `src/widgets.rs` is the host-side shortcut rendering standard. FooterKeys SDK component should produce output that the host renders through the same visual path, not a separate one.

## References

- GitHub issue #2111
- `docs/prm/app-framework-marketplace.md`
- `sdk/python/plexi_sdk/ui.py` (FooterKeys at line 790)
- `src/process_app/render.rs`

## Findings (audit outcome)

`#2111` centering was **already shipped** before this sprint (commits d8312ea6,
55ac9768, bd320e0d, f53d0041, 5f4cadce); the L1 `UiNode::FooterKeys` path in
`src/render/components.rs` centers horizontally and vertically. Confirmed, so it
was not re-touched, and #2111 is now closed with evidence.

Small-pane (<300px) component audit:
- **AppBar / Label** — fit gracefully: AppBar centers full-bleed, Label truncates
  with an ellipsis (`Text Align — 9 An…`). Verified via the 280px scene shot.
- **Column** — default `padding=SPACE_XL` (24px) costs 48px of inner width; this
  is intentional (host owns spacing) and renders fine at 280px. No change.
- **SelectList / TextInput** — truncate/scroll rather than overlap; no min-width
  failure observed. No change.
- **FooterKeys** — one real residual: the L1 path **centers but does not wrap**,
  so it overflows (not wraps) below ~300px, while `render_shortcuts` wraps but
  left-aligns. The correct fix unifies them with per-line centering + needs
  host-measured **height** feedback (the missing piece from closed #312) so
  `Column` allocates a taller band. That is architectural and out of scope here
  — filed as **#2240**.

Note: `key_combo_list` is in `src/ui/shortcuts.rs`, not `src/widgets.rs` as the
Gotchas/CLAUDE.md state (stale path). It is host-chrome only and not on the app
FooterKeys path.

## Variance

Estimate 8h, actual reflects audit + regression-lock, not a fix: the headline
(#2111 centering) was already done, so the work was verifying it, adding
`tests/scenes/footer-small-pane.toml` as narrow-pane regression evidence, closing
#2111, and filing the residual wrap debt as #2240. No production code changed —
the only correct remaining fix is architectural and belongs in #2240.

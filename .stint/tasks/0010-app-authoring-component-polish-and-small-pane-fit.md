---
id: "0010"
title: "App authoring: component polish and small-pane fit"
status: todo
estimate: "8h"
sprint: "s2"
blocked_by: []
gh_issue:
  - "2111"
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

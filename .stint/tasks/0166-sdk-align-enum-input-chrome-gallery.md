---
id: "0166"
title: "SDK fail-fast align enum, single-chrome TextInput, Canvas leaf, UI gallery app"
status: done
estimate: "6h"
completed_at: "2026-06-12T04:20:34Z"
sprint: "s2"
blocked_by: []
gh_issue:
  - "2198"
area:
  - "sdk/python"
  - "sdk/pgap"
  - "apps/examples"
tags:
  - "app-authoring"
  - "sdk-v2"
  - "fail-fast"
---


Replace the stringly-typed `ctx.text` align (silent LEFT_TOP fallback in the host) with a closed 9-value `{left,center,right}_{top,center,bottom}` vocabulary validated in the SDK and deserialized as a serde enum on the host. Rework the host TextInput to a single pill chrome (no stacked frame + glow + extra rect). Add a `Canvas(draw=fn)` SDK leaf so custom drawing needs no Component subclass. Ship a ui-gallery example app rendering every SDK component perfectly, including a 9-anchor align demo grid.

## Why

The chess POC shipped with every piece off-axis by half a glyph because `align="center_center"` silently fell back to top-left. Fail-fast turns that class of bug into a first-frame error with the fix in the message.

## References

- GitHub issue #2198
- `src/process_app/render.rs:156`
- `src/process_app/render_session.rs:185-249`
- `sdk/python/plexi_sdk/_render_context.py`
- `sdk/python/plexi_sdk/ui.py`

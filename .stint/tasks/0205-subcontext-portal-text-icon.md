---
id: "0205"
title: "Subcontext portals: text icon polish"
status: todo
estimate: "45m"
sprint: "s34"
blocked_by: []
gh_issue: []
area:
  - "ui/tile-tree"
  - "ui/chrome"
tags:
  - "v1"
  - "icons"
  - "portals"
---

Give text/editor subcontext portal previews a clearer icon instead of a generic app mark.

## Scope

- Add a compact text/document icon suitable for portal minimap and subcontext preview use.
- Prefer a paper-with-folded-corner plus short pencil overlay; keep accent color on the pencil tip/eraser/metal detail without making the icon visually long.
- Ensure the icon reads at small portal/minimap sizes and in both light and dark themes.
- Add a focused visual smoke or screenshot check if the portal preview path has existing harness coverage.

## Non-Scope

- Do not redesign all pane-type icons.
- Do not introduce bitmap assets for this small host chrome icon unless vector painting proves unreadable.

## References

- `src/spatial/tiling.rs`
- `src/file_browser/icons.rs`

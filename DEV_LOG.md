<!-- DEV_LOG.md — decision journal for the Plexi project. Newest entries at the top. Records non-obvious choices, abandoned approaches, and root causes so future sessions don't repeat mistakes. -->

## 2026-03-15 — Fix 14px black gap on right side of xterm terminal

xterm's FitAddon (v6) subtracts a scrollbar width when `scrollback > 0`: `overviewRuler?.width || 14`. With no `overviewRuler` option set, it always subtracts 14px, leaving a black gap where the canvas doesn't reach the terminal frame edge.

Fix: set `overviewRuler: { width: 1 }` in Terminal options so FitAddon subtracts 1px instead of 14px. Then hide the resulting 1px ruler canvas (`.xterm-decoration-overview-ruler`) and the native scrollbar element (`.scrollbar.vertical`) with CSS `display: none / width: 0`. Also suppress the native viewport scrollbar with `scrollbar-width: none`.

Setting `overviewRuler: { width: 0 }` doesn't work because `0 || 14 = 14` — needs a truthy value to bypass the fallback.

## 2026-03-14 — Remove overview mode entirely

Deleted the overview feature: `#overview-shell` HTML, all `.overview-*` CSS, `mode`/`camera` state, `toggleMode`/`panCamera`/`adjustZoom`/`resetViewport` from workspace-state.js, `toggleOverview`/`zoomIn`/`zoomOut` commands, all keyboard handlers, and `renderOverview`/`renderOverviewHud` functions.

Why: Overview was decorative at this stage — no dragging, no meaningful spatial navigation beyond what the minimap already provides. The mode boundary was leaky (zoom changed terminal font size even in overview mode). An empty overview state duplicated the empty landing screen. Cut it until there's a real use case.

Also fixed two pre-existing gaps exposed by the test suite: `#focus-title` was showing directory name instead of panel title, and context rename was using a custom modal instead of `window.prompt()`. Simplified rename to native prompt. Added `#toolbar-context` and `#focus-position` to the toolbar (were already tested, just missing from HTML).

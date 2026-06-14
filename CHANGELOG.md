# Changelog

Newest releases appear first.
## [0.0.783] — 2026-06-14

### Changes
- feat(ui): add shared list dropdown header
## [0.0.782] — 2026-06-14

### Changes
- feat(assistant): ESC interrupt-and-fold turn policy + cancel seam (#2204) (#2258)
## [0.0.779] — 2026-06-14

### Changes
- fix: tighten assistant thinking dots width
## [0.0.778] — 2026-06-14

### Changes
- perf: cache assistant markdown preprocessing
- feat(ai+assistant): live token/thinking streaming + assistant chat refactor (#2260)
## [0.0.777] — 2026-06-14

### Changes
- feat: ship s2 app-authoring sprint — scaffold/dev-loop, pip status, contracts + specs (#2247)
## [0.0.776] — 2026-06-14

### Changes
- fix(host/markdown): inject PlexiColors into CommonMarkViewer visuals (#2167) (#2252)
## [0.0.775] — 2026-06-13

### Changes
- feat(cli): group top-level --help into Workspace/Apps/Panes/AI/System sections (#2259)
## [0.0.773] — 2026-06-13

### Changes
- feat(cli): plexi pane info --previous N — walk back N steps in focus history (#2081) (#2256)
## [0.0.772] — 2026-06-13

### Changes
- fix(notes-picker): right-align type chips like the command palette (#2227) (#2254)
## [0.0.771] — 2026-06-13

### Changes
- plexi run: dynamic completions + extra_args forwarding (#2253)
## [0.0.770] — 2026-06-13

### Changes
- perf(ai): reduce broker snapshot and tool-loop cloning (#2028) (#2248)
## [0.0.769] — 2026-06-13

### Changes
- fix(theme): improve Nord text contrast for dim and section roles (#2249)
- feat: wire /testing gate into ship lifecycle (#2251)
- fix: context root env refresh — sidebar/overlay and CLI tip (#2250)
- feat: display agent activity status in tab bars
## [0.0.768] — 2026-06-13

### Changes
- Marketplace: hosted registry, publisher submit, paid-app license + payment stub (#2234)
## [0.0.766] — 2026-06-13

### Changes
- feat(ui): improve parked context sidebar UI and add park/unpark context menu
- feat: Ghostty-style segmented tab bar + context navigation shortcuts (#2239)
## [0.0.765] — 2026-06-13

### Changes
- fix(ui): match sidebar hover rect to selected rect size (#2238)
## [0.0.764] — 2026-06-13

### Changes
- feat(host): harden native CLI renderer builtin (#1947) (#2236)
## [0.0.763] — 2026-06-13

### Changes
- feat(assistant): Cmd+R rename, dynamic composer, scrollable picker, MockBroker tests (#2216) (#2224)
## [0.0.762] — 2026-06-13

### Changes
- feat(apps/github-issues): add label picker and smarter label chips (#2164) (#2235)
## [0.0.761] — 2026-06-13

### Changes
- feat(sdk): add PGAP shortcut helper with scene harness parity (#2196) (#2237)
- feat(ui): intensify pip working flash and stagger adjacent pips
## [0.0.760] — 2026-06-13

### Changes
- feat(cli): caller-targeted CLI — portal anchor, context push/set-root/describe, unified --from flag (#2233)
## [0.0.759] — 2026-06-13

### Changes
- perf: throttle remaining Working-pulse repaint loops to 10fps
## [0.0.758] — 2026-06-13

### Changes
- perf: event-driven frames while typing — kill the 60fps agent-pip loop
## [0.0.757] — 2026-06-13

### Changes
- fix: down-arrow newline append — no double input, no dropped repeats
- feat: unify glyph-height caret across every text input
## [0.0.755] — 2026-06-13

### Changes
- fix: closing the last pane in a subcontext collapses it and zooms out (#2232)
## [0.0.754] — 2026-06-13

### Changes
- feat: glyph-height text caret via shared draw_text_caret helper
## [0.0.753] — 2026-06-13

### Changes
- feat(palette): context-scoped pane unpacking with single-pip rows (#2228)
## [0.0.751] — 2026-06-12

### Changes
- fix: replace hand-rolled note cursor with egui's built-in caret
## [0.0.750] — 2026-06-12

### Changes
- fix scroll jitter + notes as palette entries (#2217, #2222) (#2225)
## [0.0.749] — 2026-06-12

### Changes
- fix: cursor height + scratchpad dedup (#2218, #2221) (#2226)
- fix(install): point plexiapp.com/install and README to install.sh at repo root
- feat(install): add --channel flag to one-liner install script
- Themeable status pips + SessionStart idle fix (#2219, #2220) (#2223)
## [0.0.746] — 2026-06-12

### Changes
- fix(install): offer to install Rust inline when cargo is missing
- fix(sdk): add missing Any import to ui.py; improve install.sh error handling
## Older Releases

Detailed generated notes before 0.0.746 were removed because historical tag gaps caused oversized, noisy sections. Use `git log --oneline --decorate --tags` for older commit history.

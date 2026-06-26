# Changelog

Newest releases appear first.
## [0.1.13] — 2026-06-26

### Changes
- fix(ci): skip release creation when tag already exists
## [0.1.11] — 2026-06-25

### Changes
- fix(app): refresh workspace app registry for palette
## [0.1.10] — 2026-06-25

### Changes
- feat(app): standardize SDK v3 creation flow
## [0.1.9] — 2026-06-25

### Changes
- feat: source-build update flow, persistent compile cache, release CI
## [0.1.8] — 2026-06-25

### Changes
- fix: stats app UI broken after SDK v2→v3 migration (#2328)
- fix: remove global SDK import/install fallback from app CLI (#2327)
- feat: palette search matches renamed pane titles (#2319)
- fix: website build uses stale SDK docs generator (#2325)
- Land SDK v3 as native Python app API (#2324)
- feat: app framework trust labels and bypass scan (#2317)
- fix(sdk): re-export color constants BG/FG/ACCENT/SURFACE/HIGHLIGHT/MUTED/GREEN/RED/YELLOW (#2312)
- fix: workspace-scoped config.toml ignored on initial load (#2315)
- fix: scratchpad scroll-beyond-cursor breaks after filling initial padding (#2322)
- feat: bundled agent definitions and install-time discovery (#2320)
- infra: generate PGAP capability reference docs from protocol schema (#2321)
- feat: assistant app connector protocol (#2311)
- fix(terminal): multi-line URL links broken at wrap boundary (#2310)
## [0.1.7] — 2026-06-20

### Changes
- fix: PIP overlaps pane-type chip on title+subtitle list rows (#2307)
- implement-stint: two-phase sub-agent pattern (#2306)
- infra: generate SDK reference docs from Python docstrings (#2304)
- infra: generate config reference docs from serde structs (#2305)
- feat: replace create-issue with create-stint skill, add S/M/L sizing
- infra: docs CI version accuracy and coverage checks (#2303)
- Rename notes .md file on disk when pane is renamed (#2302)
- feat: config list and config set subcommands (#2301)
- feat: add Cmd+F find bar to scratchpad text editor (#2300)
- fix: add overscroll padding to text editor (#2299)
- fix: detect one-liner wrapper install for self-update (#2298)
- feat(wasm): persistent capability grants + install trust sheet review (#2294)
- feat(wasm): zero-copy present + pong/breakout rename + WASM launch args & breakout --blocks (#2297)
- feat(file-explorer): file-handler routing + launch placement — open files into Plexi apps with OS fallback (#2283) (#2284)
- Complete wasm runtime rebuild lanes
- fix(notes): preserve scroll position across pane zoom transitions (#2293)
- fix(wasm): forward key release + canonical key names to WASM guests (#2292)
- fix(wasm): clear -D warnings build break and flaky G11 timing gate
- feat(wasm): v2 WASM component runtime end-to-end (gates G1–G7, G11–G13) (#2291)
- feat(apps): add audio-synth RT audio WASM runtime POC
- feat(apps): add bevy-pong GPU WASM runtime POC
- feat(apps): add sysmon WASM runtime POC
- feat(wit): add Plexi v2 WASM platform interface definitions
- Revert "Support direct Moss app sources"
- Revert "Add Moss Plexi proof of concept app"
- Add Moss Plexi proof of concept app
- Support direct Moss app sources
- feat(host): event subscriptions for third-party agents (#2288)
- feat(ui): one-click update button in changelog modal (#2287)
- feat(promote): auto-push version tag on just promote main
- fix(tests): prevent CI hang in notify test helper + remove dead tests
- App test scaffold + plexi app test subcommand (#2286)
- feat(promote): add optional install flag to just promote
- feat: canvas styling primitives + Stats dashboard redesign (#2285)
- fix(list): center metadata chip on two-line rows
- feat(sidebar): click parked context to unpark and switch
- fix(minimap): set icon alpha to 200 focused / 100 unfocused
- fix(minimap): widen focused/unfocused icon alpha gap to 200/60
- fix(minimap): standardize portal icon alpha across all pane kinds
- fix(website): derive docs version from Cargo.toml + audit doc accuracy
- feat(theme): add matrix, bios, plexi-night, and plexi-day presets
- fix(demo): harden event matching against user fumbles
- fix(welcome): move 'Brand new to Plexi?' hint above shortcuts
## [0.1.6] — 2026-06-20

### Changes
- feat: config list and config set subcommands (#2301)
- feat: add Cmd+F find bar to scratchpad text editor (#2300)
- fix: add overscroll padding to text editor (#2299)
- fix: detect one-liner wrapper install for self-update (#2298)
- feat(wasm): persistent capability grants + install trust sheet review (#2294)
- feat(wasm): zero-copy present + pong/breakout rename + WASM launch args & breakout --blocks (#2297)
- feat(file-explorer): file-handler routing + launch placement — open files into Plexi apps with OS fallback (#2283) (#2284)
- Complete wasm runtime rebuild lanes
- fix(notes): preserve scroll position across pane zoom transitions (#2293)
- fix(wasm): forward key release + canonical key names to WASM guests (#2292)
- fix(wasm): clear -D warnings build break and flaky G11 timing gate
- feat(wasm): v2 WASM component runtime end-to-end (gates G1–G7, G11–G13) (#2291)
- feat(apps): add audio-synth RT audio WASM runtime POC
- feat(apps): add bevy-pong GPU WASM runtime POC
- feat(apps): add sysmon WASM runtime POC
- feat(wit): add Plexi v2 WASM platform interface definitions
- Revert "Support direct Moss app sources"
- Revert "Add Moss Plexi proof of concept app"
- Add Moss Plexi proof of concept app
- Support direct Moss app sources
- feat(host): event subscriptions for third-party agents (#2288)
- feat(ui): one-click update button in changelog modal (#2287)
- feat(promote): auto-push version tag on just promote main
- fix(tests): prevent CI hang in notify test helper + remove dead tests
- App test scaffold + plexi app test subcommand (#2286)
- feat(promote): add optional install flag to just promote
- feat: canvas styling primitives + Stats dashboard redesign (#2285)
- fix(list): center metadata chip on two-line rows
- feat(sidebar): click parked context to unpark and switch
- fix(minimap): set icon alpha to 200 focused / 100 unfocused
- fix(minimap): widen focused/unfocused icon alpha gap to 200/60
- fix(minimap): standardize portal icon alpha across all pane kinds
- fix(website): derive docs version from Cargo.toml + audit doc accuracy
- feat(theme): add matrix, bios, plexi-night, and plexi-day presets
- fix(demo): harden event matching against user fumbles
- fix(welcome): move 'Brand new to Plexi?' hint above shortcuts
## [0.1.5] — 2026-06-16

### Changes
- feat(release): split version-tag push into a dedicated just release step
- fix(install): use sudo for CLI install when bin dir needs elevated access
- qa: space key routing fix, no-repeat for destructive keys, font size default, subcontext zoom-out on restore (#2280)
- polish: portal minimap icons — drop pencil, enlarge+accent document, larger terminal glyph, sidebar-sized pip
- Text-editor portal preview icon (#2279)
- feat: truncate tab titles to a single line with ellipsis (#2277)
- Automation: namespace conflict hardening (#2278)
- fix(host): text-editor/scratchpad launch creates root pane in empty context (#2276)
- Shortcuts: remove Cmd+Arrow pane navigation aliases (#2275)
- fix(sdk): sync Python SDK version with app version on every release
## [0.1.4] — 2026-06-15

### Changes
- fix(apps): align Python app venv with SDK floor
- fix(ci): pin setup-uv release tag
- fix(ci): update Node 24 compatible actions
- fix(ci): track CLI docs version source
- feat(justfile): add changelog recipe to preview unreleased changes
- feat(promote): auto-install beta after promotion instead of printing manual steps
## [0.1.3] — 2026-06-15

### Changes
- feat(sidebar): drag contexts to reorder, park, and unpark across sections (#2270)
## [0.1.2] — 2026-06-15

### Changes
- fix(theme): improve contrast on light themes — Tokyo Day, Gruvbox Light, Solarized Light
- feat(command-palette): surface commands.toml entries with hot reload (#2274)
- fix(host/context): insert subcontext after parent subtree in sidebar (#2272)
- fix: paste in Cmd+F search mode (#2271)
- v1 polish consolidation (#2268)
## [0.1.1] — 2026-06-15

### Changes
- Refresh interactive demo CLI (#2265)
- Update welcome demo prompt
- Remove host Tab app focus binding
- fix(palette): order panes and notes by recency
- fix(cli): suppress PR completions prompt
- fix(agent): refresh stale hook registrations
- feat(agent): add Codex and Pi hook installers
- Centralize host UI chrome primitives
## [0.1.0] — 2026-06-14

### Changes
- fix(palette): keep search focus after scrolling past first page
- feat(release): add stable-tier rc gates
- fix: apply workspace config on context switch (#2263)
## [0.0.785] — 2026-06-14

### Changes
- fix: include CLI docs in release commit
## [0.0.784] — 2026-06-14

### Changes
- fix: prevent changelog modal overlap
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

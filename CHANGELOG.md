# Changelog

Newest releases appear first.
## [0.2.3] — 2026-07-31

### Changes
- fix(host): drop needless borrows of &Path in adopt_python_state_orphan (#2538)
- fix(apps/todo): manifest described a state file the app never writes (#2534)
- Reclaim channel-scoped app state (#2531)
- fix(host): attribute tracebacks and quarantine pre-v3 apps (#2530)
- fix(sdk): resolve version from installed metadata too
- fix(sdk): fail loudly on missing version metadata
- fix(sdk): unify packaged Python SDK version
- fix: port repo apps to SDK v3 (#2527)
- fix(release): refuse just bump with no commits since last stable tag (#2523)
## [0.2.2] — 2026-07-29

### Changes
## [0.2.1] — 2026-07-29

### Changes
- Notification lifecycle controls (#2521)
- feat(host): add pane heartbeat timer (#2519)
- fix(host): restore fullscreen terminal input (#2518)
- fix: harden agent lifecycle and CLI transport (#2517)
- Fix scene-live warning-gated build (#2516)
- feat(editor): add soft wrapping for notes (#2515)
- feat: polish editor links and sidebar dots (#2514)
- host: guard pane state and add background launch mode (#2512)
- feat(cli): retire pane orchestration workarounds (#2511)
- feat(cli): agent orchestration verbs — pane send --submit, pane new --agent, pane slot wait (#2510)
- feat(routines): write-side CLI verbs, single schema, routine run context parity (#2509)
- fix(routines): schedule parser accepts documented syntax; lifecycle hardening; firing-path tests (#2508)
- distribution: publish plexi-cli skill via npx skills, version-lockstep release gate (#2507)
- feat(host/notifications): one enqueue choke point, audible cue, context-by-default scope (#2506)
- feat(ui/sidebar): subcontexts have no sidebar presence (#2505)
- fix(testing): test processes route away from the login keychain; would-be prompts become errors (#2504)
- feat(palette): agent panes as command palette results (#2502)
- feat(cli): plexi context sub — one command, N agent panes in a subcontext (#2500)
- feat(ai): local OpenAI-compatible broker backend for Meridian testing (#2501)
- feat(assistant): host.net.fetch URL-fetch tool behind net.http (#2498)
## [0.2.0] — 2026-07-23

### Changes
- input/notes: focus-scoped key routing kills the Tab-swallow regression class; editor UX batch (#2467)
- testing: Notes editor release gate — core/harness/installed-host matrix with seeded fuzzing (#2466)
- notes: Markdown/wiki links with keyboard activation, and dragged-image attachments with inline embeds (#2465)
- editor: code mode, Markdown transactions, and Live Preview (#2464)
- notes: drive TextEditorApp through the shared editor core (#2463)
- feat(editor): extract Ferrite-derived shared editor core (#2462)
- fix(testing): make headless guest-death probe deterministic under load (#2461)
- fix(testing): isolate HostHarness workspace root to stop ambient-state test leak (#2460)
- wip: preserve uncommitted babysitter+docs work before PR #2458 merge (#2459)
- cli: pane slot write ack + babysitter slot-first progress channel (#2458)
- Make Notes agent-drivable end to end (#2456)
- fix(ui): wrap HintBar groups at constrained widths (#2457)
- fix: make cargo tests ignore ambient Plexi state (#2455)
- ui: unify assistant composer and notification copy (#2454)
- ui: migrate rename and Secrets hints to shared primitives (#2453)
- fix(merge): self-heal worktree cleanup + stint-first pipeline path (#2450)
- cli/host: pointer-click passthrough for AppRuntime::Builtin panes (#2448)
- fix: pane capture --from-cursor empty-delta cursor accounting; SDK pytest dev-deps (#2447)
- fix: guard delete_context router-emptying cascade; add File Explorer hidden-files toggle (#2445)
- fix: punctuation-key search, Logs header inset, remove csv_viewer (#2444)
- feat(assistant): never-frozen streaming — tool-arg generation progress row + dots in every waiting state (#2443)
- fix(app-check/sdk): action probe clicks real handlers; teachable view-effect error (#2442)
- assistant: signature-complete SDK reference + placement-aware app-build skill (#2441)
- assistant build loop: fast app check, zero-discovery skill + tier floor, run UX (#2440)
- feat(assistant,host): chronological tool-call transcript + L1 TextInput typing (#2439)
- config: default-config.toml missed in the 0447 mimo rename — installer template now non-pro
- fix(render): flow-app surface fills the pane; root inset moves content only (#2438)
- fix(ai): broker resolves OPENROUTER_API_KEY from process env before keychain (#2437)
- feat(ui): monochrome emoji fallback font — chat emoji render instead of tofu (#2436)
- feat(apps): logs v2 — colored row-per-entry tail with true follow-freeze; FooterKeys standardized across maintained apps (#2435)
- config(ai): default model_medium xiaomi/mimo-v2.5-pro -> xiaomi/mimo-v2.5 (#2434)
- perf(sdk,host): diffed WASM guest→host tree updates — off-paint-thread decode (#2432)
- fix(assistant): standard chat-row layout — measured shrink-to-fit bubbles, user right-anchored (#2431)
- feat(sdk,render): token-driven default spacing — declarative apps look right with zero layout code (#2430)
- fix(apps,host): logs app reads channel log via capability-gated host effect; level-filter UI restored (#2429)
- fix(host): setsid-isolate login-shell probes from the launching terminal; route probe stderr to the channel log (#2428)
- fix(apps): breakout locks to sustainable 30fps with dt-correct physics until 0438 lands (#2427)
- fix(host): WASM panes honor Escape-to-close, overlay placement, centered load spinner (#2426)
- fix(host): first-boot context seeds base root pane via unified context-seeding helper (#2425)
- fix(assistant): left-justify wrapped text inside right-anchored user bubble (#2424)
- fix(apps): snake food truly random; balls restored to live-dims click-driven pre-L1 behavior (#2423)
- infra(install): sign bundle with stable 'Plexi Dev' identity; add just codesign-setup (#2422)
- fix(host,cli,install): stale WASM demo apps no longer hijack app names; WASM Python panes no longer drop queued clicks (#2420)
- feat(assistant): Claude Code-style app-build harness — file tools, embedded exec, diff transcript (#2419)
- fix(assistant): SDK context/tool-cap pause, tool trail collapse, composer centering, drop header (#2418)
- fix(sdk): repair or unexport broken declarative-tree widgets; add enumerate-every-export node-contract guardrail test (0413 gate follow-up)
- fix(sdk): badge/status colors validated against theme semantic roles; host decoder accepts red/green/yellow aliases; app check surfaces decode errors (0413 gate fix round 4)
- feat(assistant): host.files.read/write/edit app-dir-scoped file tools, audit-logged mutations (0413 gate fix round 3)
- feat(assistant): host.terminals.read tool, read-after-run prompt rule, tool-loop cap 10→30 with loud stop (0413 gate fix round 2)
- feat(assistant): builtin build-plexi-app skill + app-authoring prompt carve-out (0413 gate fix)
- feat: node-targeted pane click for WASM/Python app panes (#2417)
- feat(assistant): pane-open targeting + keyboard/responsive permission modal (#2416)
- assistant: interactive model picker + permissions manager (#2415)
- fix: compose remaining component-gallery widgets onto decoder-supported nodes (#2414)
- cleanup: post-WASM-consolidation debt sweep + loud cloud-execution stub (#2413)
- docs+cleanup: resolve stale L1/WIT wording, drop dead native trust-label path, retire done PRMs (#2412)
- fix: sync wasm-poc apps into profile, clarify dev-pack id-open errors (#2411)
- fix: dead-code cleanup, merge-cleanup completeness, rapid pane-key regression test (#2409)
- fix(sdk): Tabs/Grid/Toggle/ProgressBar emit decoder-unsupported node types (#2408)
- feat(host): pane click-injection primitive for canvas testing (#2407)
- fix(host): python_key_name digit mapping + Enter/return key-name mismatch (#2406)
- fix(apps): port sudoku to current SDK — canvas size + hit_region drift (#2405)
- feat(host): emit canvas-space MouseEvent on canvas click (#2404)
- fix(apps): restore sudoku to alpha + redefine core app pack (#2402)
- fix(apps): declare gpu.render + pipe.open capabilities for pong (#2401)
- fix(host): stop flooring grow-canvas height at declared height (#2400)
- feat(wasm): async staging-ring readback for GPU surface composition (#2397)
- feat(wasm): add cross-runtime events and app tools (#2396)
- feat(wasm): add Rust SDK and app scaffold (#2395)
- feat(wasm): reconcile runtime docs and complete host effects (#2394)
- feat: assistant terminal run and compact feedback (#2392)
- feat: expose Python SDK connector tools (#2391)
- fix: capture terminal tail from scrollback history (#2390)
- fix: remove dead code left by CPython-WASM migration (#2388)
- wasm: complete CPython-in-WASM migration (#2386)
- fix: route CLI commands by binary channel (#2385)
- fix: make workspace saves atomic and durable (#2384)
- assistant: add E2E harness and model verification (#2383)
- testing: complete shared host e2e loop (#2382)
- assistant: add skills and native host tools (#2381)
- assistant: add conversation history and rewind (#2380)
- feat(testing): add live TOML scene backend (#2379)
- feat(testing): expose native pane semantic state (#2378)
- feat(assistant): add agent registry and model routing (#2377)
- feat(testing): unify scene actions (#2376)
- feat: scoped Assistant settings (#2375)
- feat: add native explorer media viewers (#2366)
- feat: first-party paid-app product provisioning + real Polar fixtures (#2374)
- v1 polish: app-reported pip status SDK effect + UI gallery smoke (#2373)
- fix: host bug bundle — FooterKeys wrap, OpenArtifact symlink, event identity, cli-renderer cleanup (#2372)
- feat: Polar merchant-of-record money path — checkout, webhooks, gated downloads, 402 envelope (#2370)
- website: legal surface — ToS, privacy, refund, DMCA (#2369)
- polish: agent pips flash yellow when working, solid green when idle (#2368)
- fix: pane key drives native app keyboard handlers (#2367)
## [0.1.17] — 2026-07-08

### Changes
- feat: host plexi account CLI + PlexiAccountProvider; delete license machinery (#2365)
- feat: website account service — Postgres, magic-link auth, device-code flow (#2364)
- fix: host start windowless boot drops seeded/spawned panes (#2363)
- feat: manifest [launch] on_launch policy — focus, dedup, duplicates (#2362)
- fix: clarify merge-pr stint checkout requirement
- Bundle: Epoch 1 close (#2360)
- Bundle: stints 0343 + 0311 + 0296 + 0298 (drive-host skill, Cmd+P fix, canonical core apps, assistant stub cleanup) (#2359)
- feat(cli): plexi host start — CLI-driven host launch with declarative boot state (#2358)
- fix: reset palette scroll offset on open (#2314)
- fix: pane new --tab anchors to caller pane's window, not active (#2357)
- fix: scratchpad/notes editor scrolls to follow cursor on Enter/Tab/backspace
## [0.1.16] — 2026-07-01

### Changes
- fix: Cmd-Shift-K stops at top context instead of wrapping to bottom
- feat: refresh website visual design (#2356)
- feat: add AI onboarding guide (#2355)
- feat: route ai broker through workspace secrets (#2354)
- Expose host chrome to SDK app components
- Harden semantic app shell rendering
- Harden app shell rendering defaults
- Document app builder hot reload loop
- Return actual pane id from app open
- Harden app builder host probes
- Regenerate CLI docs for app scaffold checks
- Validate scaffold shell layout in app check
- Add semantic ActionBar app chrome checks
- Route host app actions to SDK UiAction
- Probe seeded app state in app check
- Scaffold agent app validation contract
- Fix stint claim instructions
- Update alpha app-builder coordination
- marketplace: free hosted registry smoke path (#2352)
- marketplace: scan reviewed-native bypasses (#2351)
- app distribution fixes (#2350)
- rebuild: todo app from scratch (#2349)
- redesign app UI boilerplate: action bar and footer layout (#2348)
- sdk: self-documenting flow (#2347)
- Add whats-next skill
- Correct Plexi v1 roadmap
- Refresh README app install and SDK examples (#2345)
- Revert "feat: scratchpad image URL drag-to-insert with inline preview (#2346)"
- feat: scratchpad image URL drag-to-insert with inline preview (#2346)
- fix(snake): replace deterministic food placement with true uniform random (#2343)
## [0.1.15] — 2026-06-28

### Changes
## [0.1.14] — 2026-06-28

### Changes
- fix(updater): skip sudo bin install in background build (#2339) (#2341)
- feat(promote): warn loudly if version not bumped before promoting
## [0.1.13] — 2026-06-26

### Changes
- feat(sudoku): layered cross highlight, remove 3x3 box tint, sidebar flush layout fixes
- fix(render): canvas_w/h tracks largest canvas, not last
- fix(sdk): canvas_width/height fall back to rect on first frame
- feat: add sudoku as core app (SDK v3 — HStack sidebar, hit regions, theme tokens, min_sdk_version)
- fix(render): Column layout inside HStack — wrap vertical stack in ui.vertical() and pass panel_h before horizontal context
- feat: theme token resolution in Canvas fill/color/border (stint 0309)
- feat: Canvas as L1 node in horizontal HStack layouts (stint 0308)
- feat: Canvas hit regions for click-to-element mapping (#2335)
- feat: CanvasRect border_color and border_width support (#2333)
- fix: SDK versioning unification and min_sdk_version manifest gate (#2334)
- fix(pane-ops): pane new --window uses caller context, not active context (#2332)
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

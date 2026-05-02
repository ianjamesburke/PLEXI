# Changelog

Newest releases appear first.

## [alpha] — 2026-05-02

### Changes
- feat(changelog): clickable version badge opens changelog modal + just bump-alpha (#524)
- feat(error-visibility): boot timeout + render exception re-raise (#424, partial) (#522)
- feat(sdk/ui): ListItem and Row auto-centering components (#388) (#521)
- feat(#508): chat-poc — conversational chat via AiQuery/AiResponse (#519)
- chore(#509): delete stale docs/specs/, purge dangling references (#518)
- refactor(#380): WorkspaceRouter — compile-enforced context switching invariant (#510)
- docs: clarify alpha as starting branch for all changes
- fix(egui-term): empty clipboard guard, auto-scroll boundary, HiDPI column sync (#475, #492, #472) (#504)
- feat(#425): config migration on install (#503)
- fix(#429): delete orphaned src/plexi_iq/ (#502)
- chore(promote): use --force-with-lease instead of --force for alpha→beta
- chore(promote): force-push alpha→beta to handle diverged history
- chore: promote to beta — v3.4.3
- fix(install): rename binary in bundle for non-stable channels to match config detection
- feat(commit-graph): flat N-commit load, host scroll, badge overflow fix, PR badges (#500)
- feat(logging): heartbeat watchdog + workspace autosave on structural changes (#499)
- fix(ui): hide sidebar separator line — panel is not resizable (#483) (#495)
- fix(sidebar): remove hover sense from context label — eliminates I-beam cursor (#481) (#494)
- fix(ui): Escape dismisses shortcuts overlay (#484) (#496)
- fix(ui): align shortcuts overlay — use key_combo_list for HJKL rows, add min_col_width (#482) (#498)
- feat(toolbar): show app version label next to ? button (closes #485) (#497)
- chore: sync alpha version to 3.4.2
- refactor(promote): bump+changelog on alpha before push; aggregate all entries since last tag for GitHub release
- fix: awk newline-in-variable error in prepend_changelog — use temp file + getline
- fix: bash 3.2 compat in promote.sh — replace ${var,,} with explicit y/Y check
- feat: channel promotion pipeline (just promote)
- refactor(sidebar): zone-based row abstraction with single cursor authority
- chore: set RUSTFLAGS=-D warnings globally via justfile export
- chore: remove version lifecycle, fail on warnings
- chore: untrack .channel from git index
- chore: move install logic to scripts/install.sh
- chore: unify install-alpha/beta/stable into single just install recipe
- chore: stable channel identity — name=plexi everywhere, build.rs reads .channel
- chore: derive app title from CARGO_PKG_NAME via build.rs
- chore: gitignore .channel and set per-worktree values
- docs: update CLAUDE.md, README, DEV_LOG and add icon.svg (#479)
- ci: bump actions to Node.js 24-compatible versions (#477)

## [3.4.3] — 2026-05-02

### Changes
- fix(install): rename binary in bundle for non-stable channels to match config detection
- feat(commit-graph): flat N-commit load, host scroll, badge overflow fix, PR badges (#500)
- feat(logging): heartbeat watchdog + workspace autosave on structural changes (#499)
- fix(ui): hide sidebar separator line — panel is not resizable (#483) (#495)
- fix(sidebar): remove hover sense from context label — eliminates I-beam cursor (#481) (#494)
- fix(ui): Escape dismisses shortcuts overlay (#484) (#496)
- fix(ui): align shortcuts overlay — use key_combo_list for HJKL rows, add min_col_width (#482) (#498)
- feat(toolbar): show app version label next to ? button (closes #485) (#497)
- chore: sync alpha version to 3.4.2
- refactor(promote): bump+changelog on alpha before push; aggregate all entries since last tag for GitHub release
- fix: awk newline-in-variable error in prepend_changelog — use temp file + getline
- fix: bash 3.2 compat in promote.sh — replace ${var,,} with explicit y/Y check
- feat: channel promotion pipeline (just promote)
- refactor(sidebar): zone-based row abstraction with single cursor authority

## [3.4.1] — 2026-05-01

### Changes
- **Command palette overhaul** — context/pane model rewrite; named pane entries with direct focus jump; strip stale auto window names on load
- **Remove pulse beta feature** — `pulse` config flag and breathing border effect removed
- **Fix `Cmd+Shift+,` reload shortcut** — wired through macOS menu NSEventModifierMask; was unreliable via egui key handling
- **Fix `theme_preset` TOML ordering** — was silently ignored when placed after a section header in config template
- **Remove built-in text editor** — `TextEditorApp` deleted; `Cmd+,` now opens config in system editor
- **Compile gate on `just bump`** — `cargo build --release` runs before tagging so broken builds can't reach a release
- **Square key chips** — shortcut chips resize to square for single-char keys
- **Shortcuts overlay** — two-column layout, HJKL navigation blocks, wider overlay
- **Minimap** — page numbers switched to 0-based

## [3.4.0] — 2026-05-01

### Features
- **Bundled Python 3.12** — self-contained runtime via python-build-standalone; no system Python dependency
- **Navigation stack** — `PushNav` / `PopNav` / `NavBack` protocol for multi-screen app flows
- **Async SDK** — Python SDK event loop is fully async; eliminates blocking-in-event-loop deadlocks
- **Host-managed `ScrollRegion`** — primitive for smooth scrollable app content without manual offset tracking
- **Mouse events in apps** — `PlexiEvent::Mouse*` now fires correctly inside app panes
- **`TextInput` primitive** — host-owned single-line entry with auto-focus and Shift+Enter multiline
- **Sidebar context rename** — double-click to rename; auto-rename on new context creation
- **Parallax editor app** — GUI wrapper for the Parallax video editor pipeline
- **Agent Workspace** — modal UI for spawning Claude Code agents with repo context
- **App registry** — directory-scoped app + agent discovery
- **Workspace-scoped secret routing** — secrets namespace to the active `.plexi/` workspace
- **Workspace config merge** — per-project `.plexi/config.toml` merged with global config
- **App package manager** — `install` / `uninstall` / `update` / `list` with bundled core pack
- **App lifecycle pill** — observable running/stopped indicator per app
- **OpenRouter AI backend** — configurable model tiers, real cost tracking
- **CoreMIDI I/O** — typed pipe for MIDI in/out on macOS
- **CoreAudio capture** — cpal-backed device enumeration and PCM capture
- **AVFoundation video decoder** — native macOS video playback backing
- **Hot reload** — live app reload on source change during development
- **Agent roster + inter-agent pipes** — directed communication between running agents
- **Cmd+N split-mirror + lateral focus** — `Shift+Cmd+H/J/K/L` pane navigation

### Infrastructure
- **Smart `just install`** — reads `.channel` file and dispatches to the right channel automatically
- **Canonical source identity** — `Cargo.toml` + `src/main.rs` use generic names; channel applied at build time via `sed` + restore trap; eliminates merge conflicts between alpha/beta/main
- **`.channel` + `merge=ours`** — per-branch identity file protected from merge overwrites

### Fixes
- Python SDK path corrected in `.app` bundle
- Quit freeze resolved — subscription busy-loop + child reap moved off render thread
- Empty context welcome screen on all windows
- Sidebar drag-drop reordering with visual drop indicator
- SDK clean shutdown + HiDPI hit-test alignment
- Terminal scrollback clearing on alt-screen entry

## [3.0.0-beta.5] — 2026-04-25

### Host / Notifications

- **Background app cross-context tick fix** — Global notification apps running in the background were not receiving tick events when the active pane changed context. Fix ensures the tick is dispatched correctly regardless of which context is active.

## [3.0.0-beta.4] — 2026-04-24

### Apps

- **Broken apps fixed** — `notification-tester`, `screen-time`, and `stand-up-reminder` all crashed on launch due to syntax errors or missing `Component` inheritance after the AppBar migration. All three now boot cleanly.
- **Custom component fix** — `_CountdownRing` (stand-up-reminder), `_Body` (quick-note, todo, wikipedia) now inherit `Component` so `_render_clipped` is available. Same class of bug fixed in `ui-playground`.
- **SDK bug fix** — `Scrollable.render` was orphaned outside the class body after a bad indent; moved back in.
- **Deleted lava-lamp, lava-opus, audio-recorder** — removed from examples and installed app dirs.

### SDK

- **`ensure_visible(scroll_offset, viewport_h, top, bottom, margin=0) → float`** free function in `plexi_sdk.ui`. Canonical one-line solve for selection-follows-scroll in any scrollable list. `Scrollable.ensure_visible()` wraps it as a method. Commit Graph j/k/g/G nav handlers migrated to use it.
- **`_render_clipped` on `Component`** — base class now clips every child to its allocated rect via PushClip/PopClip before calling `render`. Custom components must inherit `Component` or they will crash when placed in a `Column`.

### Host / Notification modal

- Keyboard hint row is now centered (was left-aligned due to `horizontal_wrapped` inside `vertical_centered`).
- Acknowledge button tightened: 220px → 180px, spacing above reduced from 24px → 12px.

### Host / Rendering

- `render_draw_commands` takes `pane_rect` explicitly — single source of geometry, no more `ui.min_rect()` surprise.
- Clip stack (PushClip/PopClip) intersects with current stack top so nested clips only ever tighten.

## [3.0.0-beta.3] — 2026-04-23

### Notifications — major rework

- **Pinned-by-id queue.** The currently-displayed notification is pinned by id and never displaced by incoming ones. New notifications arriving bump the count live on screen but don't yank your view to something else.
- **Priority-sorted dismiss.** Four tiers exported from `plexi_sdk`: `PRIORITY_LOW=0`, `PRIORITY_NORMAL=50`, `PRIORITY_HIGH=100`, `PRIORITY_CRITICAL=200`. After dismiss, the next front-most is chosen from whatever's in the queue *right now* — highest priority wins, arrival breaks ties.
- **Interrupt threshold (`[notifications] interrupt_threshold`).** Defaults to `100`: NORMAL and LOW queue silently (badge ticks, Cmd+Shift+A reveals), HIGH and CRITICAL auto-open the modal. Set to `0` for the old everything-interrupts behaviour; set to `201` to match `focus_mode`.
- **Esc defers, Enter acknowledges.** Previously both destroyed the notification. Now Esc closes the modal but keeps the notification in the queue (Cmd+Shift+A brings it back). Enter / option-select / input-submit still dispatches `NotifyAction` and removes from queue. Required notifications still reject Esc.
- **Cross-context scope.** New manifest field `[app] default_notification_scope = "context" | "global"`. `global` notifications (e.g. stand-up reminders) are visible regardless of active context; `context` notifications (e.g. "note saved") stay local. Apps never see scope — users control it by editing the manifest.
- **Cross-context drain.** All app panes in all contexts drain commands every frame. Previously background-context apps silently buffered notifications until you switched contexts — fixed.
- **Per-context sidebar badges.** Inactive contexts show their own context-scoped notification count.

### Apps

- **Commit Graph v2** — subway-style layout with viewport-scoped lanes (no more 18-lane noise from silent refs), hard-capped at 5 lanes with an `other` collapse bucket, fixed right-hand label column with hard truncation, hollow-diamond glyph for merge commits, parent-side colouring for non-mainline merge edges, Enter toggles tooltip, click-away clears.
- **PlexiEvent::Click forwarding** — process apps now receive pane clicks as structured events. `balls` demo gained click-to-remove.
- Six example apps migrated to SDK v2 declarative UI components (Column / Card / Header / KeyRow / FooterKeys / Section / Spacer / Footer).

### SDK + UI

- **`key_chip` primitive** (`src/widgets.rs`) — single rounded keycap pill with subtle border + monospace label. `key_combo` / `key_combo_list` wrappers built on top. Migrated every host-side shortcut hint: `?` help overlay, notification modal hint, confirm-close footer, command palette, run palette, rename-pane.
- **Python SDK `KeyRow`** — accepts `str | list[str]`, renders chips matching the host. Picked up by the Commit Graph help overlay and example-app footers.
- **`badge()` free function** + **`FooterKeys` component** in `plexi_sdk.ui`. Ends the "every app draws its own pill shape slightly wrong" class of bug. Commit Graph, todo, stand-up-reminder, wikipedia, quick-note, screen-time all migrated.
- **SDK docs pass** — new `NOTIFICATIONS` block and `MANIFEST REFERENCE` block in the module docstring covering kinds, priority guidance, queue model, scope model, `NotifyAction` round-trip, and every `[app]` manifest field.

### Host

- **GUI-bundle PATH fix** — launching Plexi from `/Applications` inherits only `/usr/bin:/bin:/usr/sbin:/sbin`. Apps shelling out to `gh` / `rg` / `fd` couldn't find them. New `shell::install_login_shell_path` resolves the user's login-shell PATH at startup and adopts it process-wide. Fallback prepends `/opt/homebrew/{bin,sbin}:/usr/local/{bin,sbin}` on macOS if the probe fails.
- **`tiling.rs` decomposed** — split into `render/{terminal,app,agent}_pane.rs`; `pane_ops.rs` split into `create / layout / workspace` submodules. Pure refactor.
- **Debug-level log targets expanded** — `plexi_alpha` and `app::<id>` targets now follow the configured log level. Previously only `plexi` / `plexi_v3` did; per-app debug/info lines were silently dropped.

### Fixes

- `git log --format` must use `%x00` / `%x01` escapes, not literal NUL bytes in argv — Commit Graph's "No commits this week" bug.
- Shebang must be line 1, not after `from __future__`.
- Spinner derives frame index from wall-clock monotonic, not per-render count.
- URL hyperlink detection across client-wrapped terminal rows.
- Drag-and-drop files onto a fullscreen pane lands in the zoomed terminal instead of silently writing to a background tile.
- Header top padding tightened (16px → 8px).
- Grey square in every pane's top-left corner (collapsing `egui::Frame` wrappers replaced with direct `ui.painter()` calls).
- Modal focus leak on Cmd+W confirm (and palette / rename) — migrated to FocusLayer with `consume_key`.
- Command palette no longer collapses to one row with a single pane.

### Justfile

- **`just clear-apps <channel>`** — explicit mirror-install helper. `cp -R` in `install-*` is sync-not-mirror; apps deleted from `examples/` don't disappear from the install dir on upgrade. Run `just clear-apps alpha && just install-alpha` for a clean slate.

## [3.0.0-beta.2] — 2026-04-23

### Added
- **Commit Graph app (github-tree, replaces file browser)** — subway-style git history viewer for the pane's repo. Vertical time, horizontal branch lanes, weekly viewport with `[` / `]` navigation, click-to-select with full message + diff-stat tooltip. No `gh` dependency — pure local git. Ships as `github-tree` (same launch slot as before).
- **PlexiEvent::Click forwarding** — process apps now receive pane clicks as structured events. `balls` demo gained click-to-remove so this is visible end-to-end.
- **SDK v2 declarative UI components** — `Column`, `Header`, `Card`, `KeyRow`, `Section`, `Spacer`, `Footer` plus design tokens (`SPACE_*`, `TEXT_*`). Six example apps migrated.
- **Multi-kind notification modal** — work-area surface for `ctx.notify`, `ctx.notify_choice`, `ctx.notify_input`, with keyboard-first navigation.

### Fixed
- **Host PATH under GUI-bundle launch** — launching from `/Applications` inherited only `/usr/bin:/bin:/usr/sbin:/sbin`, so apps shelling out to `gh` / `rg` / `fd` couldn't find them. Now resolves the user's login-shell PATH once at startup and adopts it process-wide.
- **Grey square in every pane's top-left** — collapsing `egui::Frame` wrappers in `process_app` and `tiling` have been dropped; pane backgrounds are now painted directly over `available_rect_before_wrap`.
- **Modal keyboard focus leak (Cmd+W confirm + palette + rename overlays)** — overlays now consume keys via the `FocusLayer` pipeline instead of read-only `ui.input(key_pressed)`. Hitting Enter on the close confirm no longer triggers the selected-item action in the pane behind it.
- **Command palette collapsing to one row with a single pane** — `ScrollArea` now pairs `max_height` with `min_scrolled_height`, so the viewport stays full-size regardless of content count.
- **Terminal URL hyperlink detection across wrapped rows** — client-wrapped URLs (e.g. Claude Code output) are now detected as single links spanning both rows. Cmd+click opens the full URL. Copy-path unchanged — that's tracked separately as v3.1.
- **Drag-drop on a fullscreen pane** — files dragged onto a zoomed pane now land in the zoomed terminal instead of silently writing to a background tile.
- **Screen Time rework** — 15-min buckets, clock hand, day-boundary bleed fix, SDK v2 chrome.

### Changed
- **Tiling decomposition** — `tiling.rs` split into `render/{terminal,app,agent}_pane.rs`; `pane_ops.rs` broken up into `create / layout / workspace` submodules. Pure refactor, zero behavior change.
- **Header top padding** — `Column.padding_top` default dropped from 16px to 8px so top-of-pane headers feel anchored instead of dropped.

## [1.1.2] — 2026-04-10

### Fixed
- **Cloud folder crash** — file browser no longer freezes when opening Google Drive, iCloud, or other FUSE-backed cloud folders. Eliminated per-entry `stat` syscalls in favor of cached directory entry types.
- **PTY escape query hangs** — programs like fzf that query cursor position or text area size no longer hang waiting for a response.

### Improved
- **CWD tracking performance** — cached `lsof` lookups with 300ms TTL instead of calling every frame.

## [1.1.1] — 2026-04-10

### Added
- **Theme presets** — set `theme_preset = "dracula"` (or `catppuccin-mocha`, `tokyo-night`, `gruvbox-dark`, `nord`, `solarized-dark`) in `config.toml` to apply a full UI + terminal color scheme. Individual `[theme]` overrides layer on top.
- **CRT & pulse effects** — opt-in via `[beta]` section in `config.toml`. `crt = true` adds green phosphor tint + scanlines. `pulse = true` animates the focused pane border.
- **`just install-alpha` / `just install-beta`** — build and install variant app bundles (`Plexi Alpha.app`, `Plexi Beta.app`) with fully isolated config directories (`~/.plexi-alpha`, `~/.plexi-beta`). Deprecates `just install-apps`.

## [1.1.0] — 2026-04-10

### Added
- Cmd+Comma opens config in embedded text editor.
- Inline text editing in file browser sidebar.
- Standalone text editor app.

# Changelog

Newest releases appear first.

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

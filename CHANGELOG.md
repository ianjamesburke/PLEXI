# Changelog

Newest releases appear first.

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

# Changelog

Newest releases appear first.

## [2.0.0-rc.1] — 2026-04-15

### Added
- **`OpenIntent`** — structured spawn intent on `Init`: file, url, prompt, resume, bare
- **Host event bus** — `events.jsonl` append log, `EventSubscribe`/`EventData` protocol commands
- **`Run` primitive** — `RunCreate`/`RunUpdate`/`RunComplete` draw commands, `runs.jsonl` log
- **Rich notification actions** — typed `NotificationAction` enum (Focus, Confirm, TextInput, Dismiss, ResumeRun, OpenIntent, RunCommand, ExternalUrl)
- **Protocol version negotiation** — `protocol_version` field on `Init` and manifests (default 1 for existing apps)
- **Capability enforcement** — `observes`, `create_runs`, `open_intent_kinds` manifest fields; `PermissionStore` at `permissions.json`
- **Typed pipes Phase 1** — `[app.io]` manifest section, `PipeWire` table, `PipeWrite`/`PipeListWires` draw commands
- **Plexi IQ Stage 1** — `claude -p --resume` backend, `IqSession` per pane, `PlexiIq` config struct
- **`[app.skill]`** manifest section (description, invoke_phrase)
- **`[app.agent]`** manifest section (system_prompt, tool_allowlist)
- **Python SDK 0.4.0** — `OpenIntent` class, `run_create/update/complete`, `event_subscribe`, `notify`, `pipe_write`, `on_init`/`on_event`/`on_run_created` handlers
- All bundled example apps updated to `protocol_version = 2`

## [Unreleased] — 2026-04-14

### Added
- **App infrastructure protocol v1** — the JSON line protocol between Plexi and external apps is now a stable, versioned contract. Any external Rust or Python SDK can target it without fear of breakage.
- **`plexi-sdk` 0.2.0 on PyPI** — the Python SDK is now packaged as an installable distribution (`pip install plexi-sdk`) with full metadata, README, and MIT license. Existing apps that vendor `plexi_sdk.py` continue to work unchanged.
- **Feedback primitive** — apps can now surface inline status messages (success, warning, error) through a single SDK call instead of building their own toast logic.
- **`PLEXI_APP_ID` env var** — every external app process gets its own app ID injected at spawn time, so apps can identify themselves in logs and protocol messages without registry lookups.
- **Backlog Triage app** — companion app for sweeping the `~/.plexi/backlog/` notes directory and turning loose ideas into structured action items.
- **Permissions Viewer app** — inspect which capabilities each installed app has been granted.
- **App Store update management** — version comparison against installed apps, per-app update badges, and a one-click "Update All" button.
- **Agent Ctrl+C cancellation** — interrupt an in-flight LLM request without killing the pane.
- **Claude `-p --resume` agent backend** — replaces the direct `ureq` HTTP backend with a `claude -p --resume` subprocess for dramatic cost reduction on long-running agent sessions.
- **Terminal-identity preserving app launch** — opening an app from inside a terminal now reuses the same pane via in-pane companion mode instead of spawning a new one.
- **File browser trash + undo** — `Cmd+Backspace` moves files to the trash; `Cmd+Z` restores them.
- **Shell-config app v1 spec** — design doc for a future ZDOTDIR-addon shell-configuration app (spec only — not yet implemented).

### Changed
- **Rust SDK manifest polished for crates.io publication** — full publication metadata (description, license, keywords, categories, `rust-version`) added. Version remains `0.1.0` pending protocol parity with Python 0.2.0.
- **App store URL handling** — empty paths now resolve correctly (e.g., for repos hosted at the root).

### Fixed
- **Rust example apps** — now built during `just install` so they're available immediately after install.
- **Audio Player manifest** — added missing entry field.
- **Claude binary discovery** — removed the hardcoded nvm path so the agent finds `claude` via the user's actual PATH.
- **File explorer cwd propagation** — closing the file explorer now correctly propagates the final working directory to the underlying terminal.
- **AppWithCompanion render context** — added missing `media_cache` field to fix companion-mode crashes.
- **Canonical SDK sync** — vendored `plexi_sdk.py` copies across all example apps re-synced to the canonical 0.2.0 source.

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

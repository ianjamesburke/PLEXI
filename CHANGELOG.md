# Changelog

Newest releases appear first.

## [Unreleased] — 2026-04-15

### Added
- **pyflow app** — visual Python node graph editor with editable code bodies, topological execution, and canvas pan/zoom
- **Multi-line code editor SDK primitive** — `CodeEditor` draw command for syntax-highlighted, editable code blocks in external apps
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
- **App infrastructure protocol v1** — stable, versioned JSON line protocol contract for external apps
- **`plexi-sdk` 0.2.0 on PyPI** — packaged as `pip install plexi-sdk`
- **Feedback primitive** — inline status messages via single SDK call
- **`PLEXI_APP_ID` env var** — injected at spawn time for app self-identification
- **Agent Ctrl+C cancellation** — interrupt in-flight LLM requests
- **Terminal-identity preserving app launch** — reuses same pane via companion mode
- **File browser trash + undo** — `Cmd+Backspace` + `Cmd+Z`

### Changed
- **Rust SDK manifest** — polished for crates.io (description, license, keywords, categories, `rust-version`)

### Fixed
- **Claude binary discovery** — removed hardcoded nvm path; uses user's actual PATH
- **File explorer cwd propagation** — closing correctly propagates final directory to terminal
- **AppWithCompanion render context** — fixed missing `media_cache` field crash

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

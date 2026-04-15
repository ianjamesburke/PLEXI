Always confirm best practices by researching the docs.

## GitHub Issue Labels

Every issue gets exactly one **type**, one **priority**, and one **version**. Optionally add a **status** label.

**Type** (mutually exclusive):
- **bug** — something broken
- **enhancement** — concrete improvement scoped for active development
- **idea** — speculative feature, out of scope for MVP. Use liberally — if it's not needed to ship a usable terminal multiplexer, it's an idea.

**Priority** (P1–P4):
- **P1** — MVP / shipping blocker. Fix before anything else.
- **P2** — important, not blocking. Next up after P1s are clear.
- **P3** — nice to have. Do when there's breathing room.
- **P4** — backlog / someday. Revisit when users ask for it.

**Version** (mutually exclusive, tracks which release the issue targets — match the spec file at `docs/specs/releases/plexi-vX.Y.md`):
- **v2.0** — ships in the Plexi 2.0 release (orchestration layer: OpenIntent, Runs, event bus, rich notifications, capability enforcement, typed pipes Phase 1, Plexi IQ Stage 1)
- **v2.1** — ships in 2.1 (UI primitives: viewport/transform, text_input, tabs, grid, modal, exact text measurement)
- **v2.2** — ships in 2.2 or later; systemic architectural changes that don't fit v2.0 or v2.1 cleanly

**Status** (optional):
- **in progress** — currently being worked on
- **ready** — fully researched, can be picked up immediately
- **blocked** — waiting on an external dependency or upstream fix

## Branches

Three long-lived branches, no sibling worktrees:
- `main` — stable releases. Beta → main when ready to tag a version and ship.
- `beta` — staging. Alpha → beta when a set of features is tested together.
- `alpha` — active development. **All v2 progress lands here first.** Feature branches are cut from alpha, worked in `.claude/worktrees/` subdirectories, merged back via PR.

Feature branch naming: `feature/<issue-number>-short-description` (e.g., `feature/228-run-primitive`).

Sub-agent workflow: agents use `isolation: "worktree"` to create feature branches off alpha, do their work, and open PRs targeting alpha. Never push directly to alpha, beta, or main.

## Releases

Before tagging a release (`just bump` + `just release`):
1. Update `CHANGELOG.md` at the repo root — add a new `## [x.y.z] — YYYY-MM-DD` section with a brief summary of what changed (features, fixes, breaking changes).
2. Entries are newest-first. Keep them user-facing (not internal refactor detail).

If `CHANGELOG.md` doesn't exist yet, create it with a header comment and the first entry.

## App Installation Paths

Apps are installed to a build-specific directory under `~`. The registry reads from `config_dir().join("apps")` where `config_dir` is resolved by the binary name at runtime:

| Build | Binary name contains | Apps directory |
|---|---|---|
| Alpha | `alpha` | `~/.plexi-alpha/apps/` |
| Beta | `beta` | `~/.plexi-beta/apps/` |
| Stable | anything else | `~/.plexi/apps/` |

**Always install to the correct directory for the active build.** Installing to `~/.plexi/apps/` when running the alpha build will silently do nothing — apps won't appear.

Each app is a subdirectory with a `manifest.toml` and an executable entry point:
```
~/.plexi-alpha/apps/
  wikipedia/
    manifest.toml
    wikipedia.py
    plexi_sdk.py
```

## Build & Install

`just install` uses `cargo bundle --release` to produce a proper macOS `.app` bundle (reads metadata from `Cargo.toml`), then copies it to `/Applications/Plexi.app` and extracts the binary to `/usr/local/bin/plexi`. The `install.sh` curl script does the same thing for fresh installs from GitHub.

After copying the bundle, the install recipes also run:

- `lsregister -f <bundle>` — tells Launch Services about the new bundle
- `pbs -update` — refreshes the macOS Services cache so the Finder "Open in Plexi" service (declared in Info.plist via `assets/Info.plist.ext`) appears in the right-click Services submenu without a logout cycle

If the service ever fails to show up, run those two commands manually against the installed `.app`. The `NSServices` entry is appended to the generated `Info.plist` by `cargo bundle`'s `osx_info_plist_exts` config (see `Cargo.toml` `[package.metadata.bundle]`). Validate any changes to `assets/Info.plist.ext` with `plutil -lint <bundle>/Contents/Info.plist` after running `cargo bundle`.

**After every completed code change, run the install command for the active branch:**
- `alpha` branch → `just install-alpha`
- `main` branch → `just install`

Do this before reporting a task complete so the user can immediately test in the running app.

## Logging

### Log file
Plexi writes to `~/.plexi-alpha/plexi.log` (or `~/.plexi/plexi.log` on stable). Rotates to `plexi.log.1` at startup if over 10 MB. Also printed to stderr during CLI/dev runs.

### Log level
Set in `config.toml`:
```toml
[log]
level = "info"  # error | warn | info | debug
```
Default: `info`. Use `debug` during development — it emits detailed event traces. Third-party crates (egui, wgpu, etc.) are always clamped to `warn` regardless of this setting.

### App logs (external apps → Plexi log)
External apps can forward log messages into Plexi's log file via the draw protocol. Plexi tags them with `app::<app_id>` as the log target.

**Python SDK:**
```python
# Inside a render frame (via RenderContext):
ctx.info("rendered 42 items")
ctx.warn("no data found")
ctx.error("subprocess failed")
ctx.debug("selected index: 3")

# Outside a frame (via Emitter — e.g. in on_key, on_command):
emit.info("user pressed enter")
emit.log("warn", "fallback triggered")
```

**Rust SDK:**
Emit a `DrawCommand::Log { level, message }` — the `log()` method on `RenderContext` and `Emitter` handles this.

### App stderr
External app stderr is piped and forwarded to Plexi's log as `warn`-level entries tagged `app::<app_id>`. Python tracebacks and Rust panics from external apps will appear in `plexi.log`.

### Reading logs during development
```sh
tail -f ~/.plexi-alpha/plexi.log           # live stream
grep "app::git-log" ~/.plexi-alpha/plexi.log   # filter by app
grep "ERROR\|WARN" ~/.plexi-alpha/plexi.log    # errors only
```

Sub-agents working in any worktree can read the same log file at the fixed path above.

## Lessons

- **Python version in GUI apps:** macOS GUI app bundles do NOT inherit the user's shell PATH. `#!/usr/bin/env python3` resolves to `/usr/bin/python3` (Apple's frozen 3.9.6), not the user's Homebrew 3.11+. **Always add `from __future__ import annotations` as the first line of every app Python file.** This makes `str | None` union syntax safe on Python 3.7+. Never use bare `X | Y` union types without it.
- **Install-alpha doesn't chmod:** `just install-alpha` syncs app files but does NOT set executable bits on entry points. After any install, run `chmod +x ~/.plexi-alpha/apps/*/*.py` or add this to the justfile recipe.
- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.
- **Pane focus guards:** The focus condition in `pane_ui` (tiling.rs) combines a spatial guard (`rect_contains_pointer` / `max_rect().contains(pos)`) with an intent check (click or drag). Any refactor of this condition must keep the spatial guard on every branch independently.

## General Rules

- Before starting SSH/networking setup, always ask if machines are on the same local network or remote. Before starting any multi-step infrastructure task, clarify the physical/network topology first.
- When user reports a bug or asks for a fix, focus on exactly what they asked for first. Don't pivot to QA, refactoring, or tangential improvements until the primary request is resolved.
- When user provides multiple distinct ideas, always file them as separate entries. Never combine unrelated concepts into a single item unless explicitly asked.

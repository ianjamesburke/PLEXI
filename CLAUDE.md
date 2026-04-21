Always confirm best practices by researching the docs.

## GitHub Issue Labels

Every issue gets exactly one **type** and one **priority**. Optionally add a **status** label.

**Type** (mutually exclusive):
- **bug** — something broken
- **enhancement** — concrete improvement scoped for active development
- **idea** — speculative feature, out of scope for MVP. Use liberally — if it's not needed to ship a usable terminal multiplexer, it's an idea.

**Priority** (P1–P4):
- **P1** — MVP / shipping blocker. Fix before anything else.
- **P2** — important, not blocking. Next up after P1s are clear.
- **P3** — nice to have. Do when there's breathing room.
- **P4** — backlog / someday. Revisit when users ask for it.

**Status** (optional):
- **in progress** — currently being worked on
- **ready** — fully researched, can be picked up immediately
- **blocked** — waiting on an external dependency or upstream fix

## Branch Workflow

- `alpha` — active development. All PRs land here. Use `just install-alpha` to test.
- `beta` — staging/release channel. Promoted from alpha when ready. Use `just install-beta` to test.
- `main` — stable releases only.

Feature branches are cut from `alpha`, worked in `worktrees/`, and merged back to `alpha` via PR. Never commit directly to `main` or `beta`.

Worktrees:
- `worktrees/alpha` — alpha branch
- `worktrees/beta` — beta branch

## Releases

Before tagging a release (`just bump` + `just release`):
1. Update `CHANGELOG.md` at the repo root — add a new `## [x.y.z] — YYYY-MM-DD` section with a brief summary of what changed (features, fixes, breaking changes).
2. Entries are newest-first. Keep them user-facing (not internal refactor detail).

If `CHANGELOG.md` doesn't exist yet, create it with a header comment and the first entry.

## Build & Install

`just install` uses `cargo bundle --release` to produce a proper macOS `.app` bundle (reads metadata from `Cargo.toml`), then copies it to `/Applications/Plexi.app` and extracts the binary to `/usr/local/bin/plexi`. The `install.sh` curl script does the same thing for fresh installs from GitHub.

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

- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.
- **Pane focus guards:** The focus condition in `pane_ui` (tiling.rs) combines a spatial guard (`rect_contains_pointer` / `max_rect().contains(pos)`) with an intent check (click or drag). Any refactor of this condition must keep the spatial guard on every branch independently.

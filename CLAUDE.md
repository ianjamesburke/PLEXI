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

## Worktrees

Two worktrees are set up:
- `/Users/ianburke/Documents/GitHub/PLEXI` — `main` (stable)
- `/Users/ianburke/Documents/GitHub/PLEXI-dev` — `dev` (active development)

Iterate on `dev`, merge to `main` when stable.

## Releases

Before tagging a release (`just bump` + `just release`):
1. Update `CHANGELOG.md` at the repo root — add a new `## [x.y.z] — YYYY-MM-DD` section with a brief summary of what changed (features, fixes, breaking changes).
2. Entries are newest-first. Keep them user-facing (not internal refactor detail).

If `CHANGELOG.md` doesn't exist yet, create it with a header comment and the first entry.

## Build & Install

`just install` uses `cargo bundle --release` to produce a proper macOS `.app` bundle (reads metadata from `Cargo.toml`), then copies it to `/Applications/Plexi.app` and extracts the binary to `/usr/local/bin/plexi`. The `install.sh` curl script does the same thing for fresh installs from GitHub.

## Lessons

- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.
- **Pane focus guards:** The focus condition in `pane_ui` (tiling.rs) combines a spatial guard (`rect_contains_pointer` / `max_rect().contains(pos)`) with an intent check (click or drag). Any refactor of this condition must keep the spatial guard on every branch independently.

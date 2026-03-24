Always confirm best practices by researching the docs.

## GitHub Issue Labels

- **bug** — something broken that needs fixing
- **enhancement** — concrete improvement scoped for active development
- **idea** — speculative feature, out of scope for MVP; park it and revisit when there are real users asking for it
- Use `idea` liberally to prevent backlog bloat — if it's not needed to ship a usable terminal multiplexer, it's an idea.

## Worktrees

Two worktrees are set up:
- `/Users/ianburke/Documents/GitHub/PLEXI` — `main` (stable)
- `/Users/ianburke/Documents/GitHub/PLEXI-dev` — `dev` (active development)

Iterate on `dev`, merge to `main` when stable.

## Build & Install

`just install` uses `cargo bundle --release` to produce a proper macOS `.app` bundle (reads metadata from `Cargo.toml`), then copies it to `/Applications/Plexi.app` and extracts the binary to `/usr/local/bin/plexi`. The `install.sh` curl script does the same thing for fresh installs from GitHub.

## Lessons

- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.
- **Pane focus guards:** The focus condition in `pane_ui` (tiling.rs) combines a spatial guard (`rect_contains_pointer` / `max_rect().contains(pos)`) with an intent check (click or drag). Any refactor of this condition must keep the spatial guard on every branch independently.

Always confirm best practices by researching the docs.

## GitHub Issue Labels

- **bug** — something broken that needs fixing
- **enhancement** — concrete improvement scoped for active development
- **idea** — speculative feature, out of scope for MVP; park it and revisit when there are real users asking for it
- Use `idea` liberally to prevent backlog bloat — if it's not needed to ship a usable terminal multiplexer, it's an idea.

## Build & Install

`just install` uses `cargo bundle --release` to produce a proper macOS `.app` bundle (reads metadata from `Cargo.toml`), then copies it to `/Applications/Plexi.app` and extracts the binary to `/usr/local/bin/plexi`. The `install.sh` curl script does the same thing for fresh installs from GitHub.

## Lessons

- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.

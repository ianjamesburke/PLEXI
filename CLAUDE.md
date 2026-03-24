Always confirm best practices by researching the docs.

## Build & Install

`just install` uses `cargo bundle --release` to produce a proper macOS `.app` bundle (reads metadata from `Cargo.toml`), then copies it to `/Applications/Plexi.app` and extracts the binary to `/usr/local/bin/plexi`. The `install.sh` curl script does the same thing for fresh installs from GitHub.

## Lessons

- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.

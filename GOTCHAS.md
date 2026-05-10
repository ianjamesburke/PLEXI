<!-- GOTCHAS.md — Non-obvious discoveries, failed approaches, and environment quirks specific to PLEXI. Only write an entry when something genuinely surprised you. For universal behavioral rules see ~/.claude/CLAUDE.md; for language/framework API gotchas see the coding-conventions skill. Review weekly: if the same area tag appears 3+ times, fix the root cause rather than adding another entry. -->

## Area tags: git · ship · macos · rust · egui · sdk · cargo · python · cli

---

## shell_join over-quotes single-arg terminal commands
`shell_join(["echo hello"])` produces `'echo hello'` — zsh then tries to execute a command
named `echo hello` (with the space), not `echo` with arg `hello`. Single-arg arrays are
already shell expressions; only multi-arg arrays need joining/quoting.
Use `cmd_from_args` (in `src/app/mod.rs`) everywhere a `-c` command string is built from
the terminal args array: single arg → pass as-is, multiple args → `shell_join`.

## PR build GUI won't launch when PLEXI_SOCKET is set

`open -a "Plexi PR<N>"` silently no-ops when run inside a Plexi pane because the binary detects `PLEXI_SOCKET`, prints "already running inside Plexi", and exits. Any test script that needs the PR build GUI to actually launch must either run outside Plexi (separate terminal) or `unset PLEXI_SOCKET` before the `open` call. This also affects `pkill`-and-relaunch loops — the relaunch silently fails while the script waits for a socket that never appears.

## New top-level CLI subcommands must be added to parse_workspace_path_arg SUBCOMMANDS list (cli · ship)

`src/main.rs` contains `parse_workspace_path_arg` with a hardcoded `SUBCOMMANDS: &[&str]` list. Any new top-level subcommand not in this list will be silently consumed as a workspace path argument (manifesting as "workspace path does not exist: <subcommand>"). Every new entry in `cli_args.rs` `Commands` enum must be mirrored there.

- Any code that spawns a Plexi app subprocess outside `ProcessApp::launch` must replicate its env setup: ENV_WHITELIST (HOME/PATH/LANG/LC_ALL/TERM/USER/SHELL), PLEXI_* passthrough, and PYTHONPATH → config_dir/sdk + bundle SDK path. Reference: `src/process_app/mod.rs` lines ~320–368.

## 2026-05-07 — apps dir wiped on pr-install

`~/.plexi-pr-<N>/apps/` is re-synced from `examples/` on every `just pr-install` run. Anything written directly to that directory is lost on the next install. Always put POC and test apps in the feature worktree's `examples/` directory — they will survive reinstalls and be included in the sync.

## cargo test --bin plexi for host tests (not --lib)

`cargo test --lib` only runs the `app_protocol` lib target (~47 tests). Host tests — app_registry, HostHarness, process_app, workspace_secrets — live in the binary target. Always use `cargo test --bin plexi` to run the full host test suite. `--lib` will silently pass while missing newly added registry or harness tests.

## 2026-05-05 — [ship] Uncommitted bump on alpha
When alpha's `Cargo.toml` shows a dirty version bump, `just bump` ran but failed to commit. Commit manually with `git commit -m "chore: bump alpha to X.Y.Z"` before creating a worktree — otherwise the feature branch diverges from origin at a bump commit that isn't on origin, and `gh pr merge` will fail.

## 2026-05-05 — [sdk] SDK proxy wrappers not auto-generated
`_render_context.py` contains proxy wrappers for every `Emitter` method (`notify`, `notify_choice`, `notify_input`, `notify_and_wait`, etc.). When adding parameters to `_emitter.py` methods, always update the matching proxies in `_render_context.py` in the same edit — the proxies are not auto-generated and will silently drop new params.

## 2026-05-05 — [sdk] plexi_sdk only visible to Plexi-spawned processes
`plexi_sdk` is on PYTHONPATH only for processes spawned by Plexi. A terminal pane's bare `python3` never sees it. Test SDK import changes by observing whether canvas apps open and render — not by running `python3 -c "import plexi_sdk"` in a terminal.

## 2026-05-05 — [ship] PLEXI session CWD
Sessions always start inside `worktrees/alpha/`. Running `git -C worktrees/alpha` from there fails (path doesn't exist relative to itself). Run git commands bare (`git`, `wtp`, `just`, `gh`) for alpha; use absolute paths (`/Users/ianburke/Documents/GitHub/PLEXI/worktrees/feature/<branch>/`) for feature worktrees.

## 2026-05-05 — [git] git index false-dirty
`git -C <path> status --porcelain` can show modified files when `git diff HEAD` is empty — the index timestamps are stale, not actual changes. Run `git update-index --refresh` before treating the branch as dirty.

## 2026-05-05 — [macos] SVG previews via CLI
`open -a Preview file.svg` is unreliable when called from a CLI context. Use `rsvg-convert -w 400 -h 400 in.svg -o out.png` and open the PNG instead.

## 2026-05-05 — [ship] Worktree CWD gone after wtp remove
After merging and running `wtp remove`, the feature worktree directory no longer exists. Any shell commands that reference it will fail. Always finish all file edits and cd away before cleanup steps.

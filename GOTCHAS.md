<!-- GOTCHAS.md — Non-obvious discoveries, failed approaches, and environment quirks specific to PLEXI. Only write an entry when something genuinely surprised you. For universal behavioral rules see ~/.claude/CLAUDE.md; for language/framework API gotchas see the coding-conventions skill. Review weekly: if the same area tag appears 3+ times, fix the root cause rather than adding another entry. -->

## Area tags: git · ship · macos · rust · egui · sdk · cargo · python · cli · quick-note · railway

---

## [cli] New top-level subcommands must be added to SUBCOMMANDS in parse_workspace_path_arg

`parse_workspace_path_arg` in `src/main.rs` runs on raw args before clap parses anything. It has a hardcoded `SUBCOMMANDS: &[&str]` allowlist. Any first positional arg not in that list is treated as a workspace path, giving `error: workspace path does not exist: <name>` even when the subcommand is defined in the `Commands` enum.

**Fix:** add the lowercase subcommand name to `SUBCOMMANDS` in `parse_workspace_path_arg` (line ~601 in `src/main.rs`) for every new `Commands` variant.

---

## [git · ship] Unpushed alpha commits are silently lost when ship-issue agents rebase

`ship-issue` runs `git pull --rebase origin alpha` at Phase 1. If there are commits on the local alpha branch that haven't been pushed to origin, the rebase replays them on top of origin's HEAD — but if those commits touch files that were also changed by merged PRs (e.g. skill files, CLAUDE.md), they will conflict and be silently dropped or overwritten.

**What NOT to do:** commit directly to alpha and leave without pushing. Any agent that starts a ship cycle will nuke those commits.

**Fix in ship-issue skill:** Phase 1 now checks `git log origin/alpha..HEAD --oneline` before rebasing and hard-stops if unpushed commits exist.

**Rule:** every direct commit to alpha must be followed immediately by `git push origin alpha`. No exceptions.

---

## [egui] TextEdit focus in complex modal frames — two-layer focus problem

`ctx.memory_mut(|m| m.request_focus(id))` is **last-caller-wins within a single frame**. Modal overlays face a two-layer focus problem:

**Layer 1 — intra-overlay contest:** Rendering other interactive widgets (ScrollArea, Buttons, pane rows) AFTER `request_focus()` overwrites the request. Any widget that renders after can steal focus.

**Layer 2 — frame-order contest:** Overlays dispatch during "early overlay dispatch" in `update()`, BEFORE `CentralPanel` renders. App panes render their own TextInput widgets during CentralPanel, calling `request_focus()` on them — which then overwrites the overlay's earlier request. The overlay's TextEdit never receives input.

**Wrong pattern:**
```rust
if !te.has_focus() { te.request_focus(); }  // re-every-frame in the overlay render fn — CentralPanel panes steal it back
```

**Correct pattern — two fixes, both required:**

**Fix 1 (one-shot, for Layer 1):** Add a `*_focus_requested: bool` flag. After ALL widgets in the overlay render, call `request_focus` exactly once, then set the flag. Gating on `!focus_requested` prevents re-firing. Also set cursor selection state here (use `chars().count()` not `len()` for multi-byte safety).

**Fix 2 (every-frame, for Layer 2):** In `update()`, after CentralPanel completes (search for the `palette_search` / `quick_note_text` re-focus block), add:
```rust
if self.<overlay>_renaming {
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("<overlay>_rename_input")));
}
```
This mirrors the established pattern for `palette_search` and `quick_note_text`, which face the same frame-order problem.

Both fixes are necessary. Fix 1 alone fails when CentralPanel runs after the overlay. Fix 2 alone skips initial cursor selection.

Canonical implementation: `inspector_rename_focus_requested` (one-shot, `src/overlays.rs::draw_context_inspector`) + `if self.inspector_renaming { ctx.memory_mut... }` block in `src/app/mod.rs::update()` after the QuickNote re-focus block.

---

## [macos · rust] proc_listchildpids(NULL, 0) returns EFAULT on macOS 23.x

`proc_listchildpids` with a NULL buffer and 0 size is documented to return the bytes needed (n_children × sizeof(pid_t)). On macOS 23.x (Sonoma) it returns -1 (EFAULT) instead. Any code that treats a negative return as "default busy" will show every shell as busy. Use `pgrep -P <pid>` instead — exits 0 when children exist, 1 when idle, and is reliable across all macOS versions. The 500ms cache in `shell::has_foreground_child` keeps the subprocess overhead acceptable.

---

## [railway] Build context must be repo root, builder must be Dockerfile

The website Dockerfile (`website/Dockerfile`) uses `COPY website/ .` and `COPY sdk/python/plexi_sdk /sdk/plexi_sdk` — both paths are relative to the build context root. Three things must align in Railway dashboard for this to work:

1. **Root Directory** = `/` (or blank) — so the build context includes the entire repo, not just `website/`
2. **Builder** = `Dockerfile` (not Railpack) — Railpack auto-detects Rust from `Cargo.toml` at repo root and tries to build the host binary instead of the website
3. **Dockerfile Path** = `website/Dockerfile`

`railway.json` at repo root declares `"builder": "DOCKERFILE"` but Railway's dashboard setting takes precedence over `railway.json` when Railpack is explicitly selected. If the build log shows `railpack` or Rust edition errors, the builder override isn't active.

**Symptoms by misconfiguration:**
- Root Directory = `website/` → `"/website": not found` (COPY looks for `website/` inside `website/`)
- Builder = Railpack → Rust `edition2024` errors (Railpack found `Cargo.toml`, ignoring Dockerfile)
- Both wrong → either symptom depending on which fails first

## [quick-note] shell injection surface for pane destination token substitution
`substitute_note_tokens_static` applies `shell_quote` to both `{note}` and `{cwd}` before substituting into the command template. `shell_quote` uses POSIX single-quote wrapping (`'...'`) with `'` escaped as `'\''` — this blocks all expansion inside the quoted region including `$(...)`, backticks, `\`, newlines, and semicolons. Audit (issue #1113) confirmed no gaps: all three call sites (pane_ops/create.rs and two in overlays.rs) route through the same function.

Key constraint: the command *template* comes from `config.toml` (user-controlled, not free-form input). Only the values substituted at `{note}` and `{cwd}` are escaped — the template structure itself is trusted. Do not add new substitution tokens that expand arbitrary strings without routing them through `shell_quote`.

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

`~/.plexi-pr-<N>/apps/` is re-synced on every `just pr-install` run. Anything written directly to that directory is lost on the next install. Always put POC and test apps in the feature worktree's `apps/dev/` directory — they will survive reinstalls and be included in the alpha/PR sync. Production apps (core and examples) go in `apps/` at the top level.

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

## 2026-05-27 — [egui · macos] `Modifiers::COMMAND` is Ctrl off-macOS, so host shortcuts shadow terminal control codes
egui maps `Modifiers::COMMAND` to ⌘ on macOS but to **Ctrl** on Windows/Linux. Plexi's host shortcuts are all defined with `cmd()` (= `COMMAND`), so on non-mac every shortcut became a bare `Ctrl+<key>` chord — the exact namespace terminals use for control codes (Ctrl+I = Tab, Ctrl+R = reverse-search, Ctrl+W = delete-word, Ctrl+[ = ESC, …). `poll_actions` runs before the terminal view reads input and `consume_key`s the event, so the focused terminal (neovim, readline) never received it.

**Fix (`src/keys.rs`):** on non-mac the modifier tiers shift up — `cmd()` → Ctrl+Shift, `cmd_shift()` → Ctrl+Alt — leaving bare Ctrl free for the terminal. macOS keeps the ⌘ scheme. Both paths are `#[cfg]`-split in the `cmd*()` helpers.

**Subset-matching trap:** egui's `consume_key` uses `Modifiers::matches_logically`, which treats shift/alt as a *subset* — a `Ctrl+Shift` pattern also matches a `Ctrl+Shift+Alt` press (but not vice-versa). Tiers MUST therefore be checked most-specific-first in `poll_actions`: `cmd_shift` (Ctrl+Shift+Alt) before `cmd` (Ctrl+Shift) and before `cmd_ctrl`/`cmd_alt` (Ctrl+Alt). Adding a binding on a key that already has a shifted sibling requires placing the more-specific check first, or it silently eats the other. This is why swap-pane is checked after tabs, and reload-config before open-config.

**Display:** shortcut hints must build combos from `widgets::MOD_CMD` / `MOD_CMD_SHIFT` (which encode the per-platform chip sequence) via `widgets::combo()`, not hand-composed `CMD`+`SHIFT`, or Windows help text under-reports the modifier.

**Shift mutates punctuation/digit logical keys.** Once shortcuts include Shift, egui no longer reports the base punctuation key: egui-winit sets the event key to `logical_key.or(physical_key)` (egui-winit lib.rs:764), and `Shift+/` has logical key `?`. So `consume_key(Ctrl+Shift, Key::Slash)` never matches `Ctrl+Shift+/` — it arrives as `Key::Questionmark`. This breaks exactly the keys egui exposes a shifted variant for: `/`→`?`, `[`→`{`, `]`→`}`, `\`→`|`, `=`→`+`, `1`→`!`. Keys with NO variant (`,`→`<`, `-`→`_`, digits `2`–`9`/`0`) fall through to their physical key and still match. Fix: `shift_variant(base, shifted)` in `keys.rs` binds the shifted variant on non-mac (US/ANSI layout assumed). Symptom when missed: the shortcut silently does nothing (reported for `Ctrl+Shift+/`). Letters are unaffected — `Shift+i` is still `Key::I`.

<!-- GOTCHAS.md — Non-obvious discoveries, failed approaches, and environment quirks specific to PLEXI. Only write an entry when something genuinely surprised you. For universal behavioral rules see ~/.claude/CLAUDE.md; for language/framework API gotchas see the coding-conventions skill. Review weekly: if the same area tag appears 3+ times, fix the root cause rather than adding another entry. -->

## Area tags: git · ship · macos · rust · egui · sdk · cargo · python · cli · quick-note · railway

---

## [cargo · ship] Changing target-dir causes silent install failures

All repo worktrees share a single build cache via `[build] target-dir` in `.cargo/config.toml` (or `CARGO_TARGET_DIR`). `scripts/install.sh` resolves the bundle path by calling `cargo metadata` at install time so it always points at the canonical target directory.

**If you change the target-dir setting, install.sh automatically adapts.** But if `cargo metadata` fails (Python 3 missing, not in a cargo workspace, etc.), the script now exits immediately with a clear error rather than falling back to `"target"` and silently installing nothing.

**Root cause of original failure (d21a8d0f):** the script previously hardcoded `"target"` as a fallback, so when the actual target dir was different (e.g. a shared sibling path), `cargo bundle` built to the real location but the copy step silently found nothing and exited 0. Fixed by: (a) making the metadata lookup non-optional (empty result = immediate failure), and (b) naming the exact path in the bundle-not-found error so engineers can diff expected vs. actual.

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

## Skill file edits don't need bump + install

When the only change is to `.claude/skills/*.md` or similar non-Rust config, just commit directly to alpha. `just bump && just install` is only needed when there's a code change that should be reflected in the running build. Bumping for a skill file edit creates an unnecessary version tag.

## [cli] Path-based app commands must not call resolve_workspace_root

`app validate <path>`, `app install <path>`, and `app run <path>` operate on an
explicit filesystem path — they must never call `resolve_workspace_root` (or
`require_workspace`) for their primary path argument. Workspace resolution silently
returns `None` when no `.plexi/` ancestor exists, but any code that treats `None` as
an error will break agents working inside a cloned repo with no workspace.

**Correct pattern:** use the path argument directly (canonicalize via `std::fs::canonicalize`
or `std::path::Path::new(path)`). No workspace lookup needed.

**Wrong pattern:** walking up from the path arg with `resolve_workspace_root` to derive
the app dir. That inverts the semantics — the user is *providing* the app dir, not a
workspace that should contain it.

`resolve_workspace_root` is legitimate inside `AppRegistry::load` (to surface
workspace-local apps) and in `app init` (to decide where to scaffold). In those cases
`None` gracefully degrades (falls back to global) rather than hard-failing.


## `just pr-install` must run from the feature worktree

`scripts/install.sh` derives `REPO_ROOT` from `${BASH_SOURCE[0]}/..`. Running from the repo root resolves to alpha's working tree, so `rsync -a apps/dev/` syncs alpha's `apps/dev/` — missing any apps that only exist on the feature branch. Always `cd worktrees/feature/<branch> && just pr-install <N>`.

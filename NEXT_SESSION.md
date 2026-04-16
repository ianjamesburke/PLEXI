# Parked V2 Experiments — Cherry-Pick Guide

After the 2026-04-15 cleanup, `alpha` is the single source of v2 truth and is stable for incoming PRs.

**Resolved so far (2026-04-15 session 2):**
- `experiments/v2-spawn-app` — subsumed, deleted from origin ✓
- `experiments/v2-sdk-breakpoints` — subsumed, deleted from origin ✓
- `experiments/v2-secrets-get-api` — subsumed (SecretGet on alpha), deleted from origin ✓
- `experiments/v2-scope-spec` — never pushed, gone; CODEOWNERS at `.github/CODEOWNERS` ✓
- `experiments/v2-plexi-iq-stage0` — never pushed, gone; `src/plexi_iq/` already on alpha ✓
- `docs/types/core/*.toml` — already on alpha ✓
- Secrets manager proposal — filed as issue #247, spec at `docs/specs/proposals/secrets-manager.md` ✓

**Remaining on origin (2 branches):**

**Do NOT `git merge` these wholesale.** Conflict surface is massive and most commits are already shipped on alpha via different paths.

## Branch inventory

### `experiments/v2-slash-commands-spawn` (still active — WIP source code not yet extracted)

Most doc/example content already extracted or confirmed on alpha. **One remaining item needs hands-on review:**

**WIP checkpoint `c7d9cc3`** — `src/agent_mode.rs` (+344), `src/pane.rs` (+7), `src/pane_ops.rs` (+2), `src/tiling.rs` (+98). This is the slash-command trigger (`/` at empty prompt → agent mode) + tiling layout changes. Alpha's `agent_mode.rs` is 424 lines; this WIP adds 344 more. Do NOT blind cherry-pick — diff against current alpha first, extract only the slash-command/intent-trigger logic onto a fresh `feature/104-slash-trigger` branch. File as issue before starting.

**Already extracted/confirmed:**
- `docs/types/core/*.toml` — on alpha ✓
- `docs/specs/plexi-iq.md` — extracted to `proposals/plexi-iq.md` ✓
- `docs/specs/SKILL.md`, per-app tests, photo-viewer, spiral-viewer — on alpha ✓
- spawn_app, breakpoints — subsumed ✓

### `experiments/v2-external-text-editor` (defer until v2.1)

Cross-language Python + Rust text editor example. Only unique commit: `e6f5220`. Useful as Tier 3 reference app once v2.1 `ctx.text_input` primitive lands. **Do not touch until v2.1 is scoped.**

### `experiments/v2-input-layering` (+8, was `feature/v2-input-layering-contract`)
Input layering spec (which layer gets a keystroke first) + SecretGet API + an earlier `protocol-v2.md` scope doc.

**Unique value:**
- Input layering section — extract as `docs/specs/proposals/input-layering.md` if not already covered by `plexi-v2.0.md` §7 capability enforcement.
- SecretGet API may already be on alpha; verify before re-adding.

### `experiments/v2-plexi-iq-stage0` (+7, was `feature/237-file-explorer-75-split`)
Despite the old name, the real value is `src/plexi_iq/` Stage 0 scaffolding (LlmBackend trait, module tree). Foundational for issue #210/#211/#212 (Plexi IQ tracking).

**Unique value:** The `src/plexi_iq/` module stub. Cherry-pick as the start of Plexi IQ Stage 1 implementation (#231).

### `experiments/v2-scope-spec` (+5, was `feature/plexi-v2-scope-spec`)
Older scope doc + CODEOWNERS. Mostly superseded by current `docs/specs/releases/plexi-v2.0.md`.

**Unique value:** CODEOWNERS file, if not already present. Otherwise delete this branch.

### `experiments/v2-spawn-app` (+19, was `feature/spawn-app-protocol`)
Likely fully subsumed by `experiments/v2-slash-commands-spawn` — the audit flagged these as duplicate snapshots. Verify with `git log experiments/v2-slash-commands-spawn..experiments/v2-spawn-app --oneline` before cherry-picking. If empty delta, delete.

### `experiments/v2-sdk-breakpoints` (+19, was `feature/sdk-breakpoints-min-size`)
Same situation as `experiments/v2-spawn-app` — likely duplicate of `experiments/v2-slash-commands-spawn`'s breakpoint work. Verify and delete.

### `experiments/v2-secrets-get-api` (+2, was `worktree-agent-a364e07d`)
SecretGet app API request + Python/Rust SDK exposure. May be partially covered by `experiments/v2-input-layering`.

**Unique value:** Clean SecretGet implementation worth extracting if not already on alpha. Verify first — search for `SecretGet` in `src/app_api.rs`.

## Recommended order

1. **`experiments/v2-plexi-iq-stage0`** — cherry-pick `src/plexi_iq/` stub onto a fresh `feature/231-plexi-iq-stage1` branch off alpha. Unblocks #231.
2. **`experiments/v2-slash-commands-spawn`** — cherry-pick `docs/types/core/*.toml` onto a fresh `feature/226-typed-pipes-core-kinds` branch off alpha. Unblocks #226.
3. **`experiments/v2-slash-commands-spawn`** again — cherry-pick the per-app `tests/test_*.py` files if the current alpha doesn't have them.
4. **`experiments/v2-secrets-get-api`** — verify SecretGet isn't on alpha; if missing, cherry-pick.
5. **`experiments/v2-input-layering`** — extract input layering section into a proposal doc.
6. Verify `experiments/v2-spawn-app` and `experiments/v2-sdk-breakpoints` are subsumed by the slash-commands-spawn branch; delete both.
7. `experiments/v2-scope-spec` → extract CODEOWNERS if missing, then delete.
8. `experiments/v2-external-text-editor` → preserve until v2.1 lands `ctx.text_input`; then use as Tier 3 reference.

## v2.0 RC (PR #249) — bugs found during testing (2026-04-15)

Worktree: `.claude/worktrees/agent-ad8a1f5b`, branch: `worktree-agent-ad8a1f5b`

Fix these before merging to alpha:

1. **No shebang on Python example apps** — all `.py` entry points installed to `~/.plexi-alpha/apps/` are missing `#!/usr/bin/env python3`. Shell executes them as bash → immediate crash. Fix: add shebang to every example app entry point, or add `chmod +x` + shebang enforcement to `just install-alpha`.

2. **Event bus emits nothing** — `events.jsonl` is created (0 bytes) but no events are written during a session. `EventLog::emit` calls in `process_app.rs` are either not being called or not reaching the background writer task. Likely cause: `AppState` field not being threaded through to the draw-command handlers, or the tokio task in `EventLog::new` is spawned outside the active runtime.

4. **Ctrl+/ agent mode regression** — Ctrl+/ no longer opens the agent mode overlay. Likely same root cause as #3 — keybinding conflict or the agent mode wiring to IQ broke the shortcut handler.

3. **Cmd+Shift+N keybinding regression** — notification palette no longer opens on Cmd+Shift+N. Opens a new context instead. A new keybinding added in the v2 RC is stealing the shortcut. Grep for `Shift+N` / `shift_n` in the worktree diff vs alpha to find the conflict.

---

## When to delete each experiment branch

As soon as its unique pieces are cherry-picked onto a clean feature branch off alpha and verified building. `git branch -D experiments/v2-<name>`. Remote tracking branches untouched — push deletions explicitly with `git push origin --delete` only if you want the GitHub UI cleaned up.

**Delete this file** once all 8 branches are resolved. It has no reason to exist after that.

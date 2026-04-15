# Parked V2 Experiments — Cherry-Pick Guide

After the 2026-04-15 cleanup, `alpha` is the single source of v2 truth and is stable for incoming PRs.

**Eight `experiments/v2-*` branches are parked locally** — they contain pre-reorg work too tangled with the old SDK layout (1640-line vendored `plexi_sdk.py` copies) and the old flat `docs/specs/` layout to mass-merge into current alpha. Cherry-pick valuable pieces onto fresh feature branches off alpha. Delete each experiment branch once its unique value is extracted.

**Do NOT `git merge` these wholesale.** Conflict surface is massive and most commits are already shipped on alpha via different paths.

## Branch inventory

### `experiments/v2-slash-commands-spawn` (+36, was `feature/104-slash-trigger-and-commands`)
The tangled heavyweight — 36 commits including one WIP checkpoint committed during the cleanup (`c7d9cc3`). Mixes work already shipped on alpha (spawn_app Phase 0, typed-pipes Phase 0, breakpoints) with genuinely new material.

**Unique value worth cherry-picking:**
- `docs/types/core/*.toml` — seed core type registry for typed pipes (text / json / file_path / selection / event / metric). Required for typed pipes Phase 1 (v2.0, issue #226-equivalent).
- `docs/specs/plexi-iq.md` — Plexi IQ spec draft. Reconcile with current `docs/specs/subsystems/agent-orchestration.md` and `docs/specs/releases/plexi-v2.0.md` §9.
- `examples/{app-store,calc,clipboard-stack,color-palette,json-viewer}/tests/test_*.py` — per-app test suites using `plexi_test.py` harness.
- `examples/parallax/manifest.toml`, `examples/audio-player/manifest.toml` — additional example app manifests.
- **WIP checkpoint `c7d9cc3`** — uncommitted slash-command/tiling changes: `src/agent_mode.rs` (+344), `src/pane.rs` (+7), `src/pane_ops.rs` (+2), `src/tiling.rs` (+98). No claim these compile against current alpha; review before extracting.

**Skip:**
- Old vendored `plexi_sdk.py` copies (alpha uses symlinks now; SDK deploy Phase 2 is #244).
- Old `docs/specs/*.md` flat-layout edits (alpha has the three-bucket reorg).
- Already-shipped spawn_app / breakpoints / text-editor work.

### `experiments/v2-external-text-editor` (+19, was `feature/external-text-editor-app`)
Cross-language SDK composition + real editor improvements (find/replace, syntax, undo, autosave, status bar). May be partially subsumed by `experiments/v2-slash-commands-spawn`.

**Unique value:** Cross-language Python + Rust example pair proving SDK composition. Useful as a Tier 3 reference app once v2.1 `ctx.text_input` primitive lands.

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

## When to delete each experiment branch

As soon as its unique pieces are cherry-picked onto a clean feature branch off alpha and verified building. `git branch -D experiments/v2-<name>`. Remote tracking branches untouched — push deletions explicitly with `git push origin --delete` only if you want the GitHub UI cleaned up.

**Delete this file** once all 8 branches are resolved. It has no reason to exist after that.

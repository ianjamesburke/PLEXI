# Next Session — Preserved V2 Branches

After the 2026-04-15 branch/worktree cleanup, `alpha` is the single source of v2 truth. Everything on alpha is v2-relevant and current.

**Six feature branches were preserved** for later review rather than mass-merged, because their work is valuable but their commit histories are tangled against the freshly-reorganized `alpha` (SDK symlinks, spec reorg, components layer) and mechanical merges would conflict badly.

Each branch below is a candidate for cherry-pick OR re-implementation on a fresh branch off alpha. Do NOT merge any of these wholesale — inspect the commits, extract the unique value, and throw the rest away.

## Preserved branches

### `feature/104-slash-trigger-and-commands` (+35 commits)
**The tangled heavyweight.** Mixes already-shipped work (spawn_app, typed-pipes Phase 0, breakpoints) with genuinely new material. Worktree: `.claude/worktrees/agent-ae3ad3ec` (dirty — has real `src/agent_mode.rs` changes).

**Unique value to extract:**
- `docs/types/core/*.toml` — seed core type registry for typed pipes (text / json / file_path / selection / event / metric). Needed for typed pipes Phase 1 (v2.0, #226-equivalent path).
- `docs/specs/plexi-iq.md` — Plexi IQ spec draft. Reconcile against `docs/specs/subsystems/agent-orchestration.md` and `docs/specs/releases/plexi-v2.0.md` §9.
- Per-app test suites: `examples/{app-store,calc,clipboard-stack,color-palette,json-viewer}/tests/test_*.py`
- `examples/parallax/manifest.toml`, `examples/audio-player/manifest.toml` — additional example apps
- `src/agent_mode.rs` changes — uncommitted in the worktree; review before deleting the worktree

**Skip:**
- Old vendored `plexi_sdk.py` copies (alpha uses symlinks now — #244 Phase 2 is the proper fix)
- Old `docs/specs/*.md` flat-layout edits (alpha has the three-bucket reorg now)
- Any spawn_app / breakpoints / text-editor improvements — those appear to already be on alpha via other paths

### `feature/external-text-editor-app` (+19 commits)
Cross-language SDK composition + real editor improvements (find/replace, syntax highlighting, undo, autosave, status bar). Might be partially subsumed by feature/104.

**Unique value:** Cross-language (Python + Rust) example pair proving SDK composition. Needed as a Tier 3 reference app when v2.1 text_input primitive lands.

### `feature/v2-input-layering-contract` (+8 commits)
Input layering spec (which layer gets a keystroke first) + SecretGet API + an earlier-version `protocol-v2.md` scope doc.

**Unique value:**
- The input layering section — extract as a new proposal doc at `docs/specs/proposals/input-layering.md` if it's not already covered by the current `plexi-v2.0.md` §7 capability enforcement.
- SecretGet API may already be on alpha; verify before re-adding.

### `feature/237-file-explorer-75-split` (+7 commits)
Includes `plexi-iq` Stage 0 scaffolding (LlmBackend trait, module tree in `src/plexi_iq/`). Foundational for issue #210/#211/#212 (Plexi IQ tracking).

**Unique value:** The `src/plexi_iq/` module stub. Worth cherry-picking as the start of the Plexi IQ Stage 1 implementation (#231).

### `feature/plexi-v2-scope-spec` (+5 commits)
Older scope doc + CODEOWNERS. Mostly superseded by the current `plexi-v2.0.md`.

**Unique value:** CODEOWNERS file, if not already present. Otherwise delete.

### `feature/spawn-app-protocol` (+19 commits) and `feature/sdk-breakpoints-min-size` (+19 commits)
Audit reported these are likely fully subsumed by feature/104 (same commits via a different branch). Verify with `git log` comparison before cherry-picking from one vs the other. If confirmed subsumed, delete both.

### Also preserved: `worktree-agent-a364e07d`
Ties to a worktree with 19 build artifacts (`sdk/rust/target/`). Branch contains SecretGet API work (c891eb3). Worktree path: `.claude/worktrees/agent-a364e07d`. Review and either extract the SecretGet commit or delete both the branch and the worktree.

## What's live

- `alpha` branch (local + origin) is at `409fb37` — has every v2 commit from this session: notifications urgency model, Protocol v2 spec, Protocol v2.1 spec, Phase 1 components layer, Tier 1 app fan-out, launch/Escape fixes, SDK symlink cleanup, docs reorg.
- 15 tracking issues on GitHub labeled by version (`v2.0` / `v2.1` / `v2.2`) — see #224 umbrella for the v2.0 release index.
- `main` / `beta` untouched (remote is source of truth).

## Recommended next steps

1. Look at the `feature/104` uncommitted `agent_mode.rs` changes in `.claude/worktrees/agent-ae3ad3ec/`. Decide keep/trash.
2. Cherry-pick `docs/types/core/*.toml` from feature/104 onto alpha (or a fresh feature branch) for v2.0 typed pipes Phase 1.
3. Cherry-pick `src/plexi_iq/` stub from feature/237 onto alpha for v2.0 Plexi IQ Stage 1 start.
4. Delete feature/spawn-app-protocol and feature/sdk-breakpoints-min-size if confirmed subsumed by feature/104.
5. Push alpha to origin so the remote catches up with the local fast-forward.
6. Delete this file once the six preserved branches are resolved.

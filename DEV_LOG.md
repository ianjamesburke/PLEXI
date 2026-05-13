# DEV_LOG

Development log for PLEXI. Tracks root causes, non-obvious decisions, abandoned approaches, and environment quirks that git history won't capture. Entries are newest-first — most recent at the top.

## 2026-05-13 — [CHANGED] Self-documenting SDK — AST-based doc generator (PR #1258 → alpha)

Added `website/scripts/generate-sdk-docs.py` that parses SDK Python source via `ast` module and generates 5 Astro markdown pages (overview, App, RenderContext, Emitter, Widgets). Wired into `npm run build` so docs regenerate on every deploy. Docker build context moved from `website/` to repo root so Dockerfile can access both `website/` and `sdk/`.

Process revealed several SDK-side inconsistencies (filed as #1260): stale handler overview listing 12 of 26+ handlers, notify priority signatures lying to type checkers (`int | None = None` but raises TypeError on None), spawn handler naming breaking underscore convention, three dataclasses with zero docstrings. Generator also needed to filter `@property` getters/setters and strip rST `::` artifacts.

**Breaks if:** `npm run build` in `website/` fails (generator must run before Astro); or Railway deploy can't find SDK source (build context must be repo root, `SDK_SOURCE=/sdk/plexi_sdk`).

## 2026-05-11 — [CHANGED] Config template made fully self-documenting (PR #1117 → alpha)

`CONFIG_TEMPLATE` in `src/config.rs` was a sparse file with minimal comments. Replaced with a fully documented template: notifications values commented-out with tier docs, `[theme]` compact block with docs link and catppuccin-mocha defaults, `[ai]` fully commented as "coming soon", keybindings all commented-out, and 3 active Quick Note defaults (Backlog, Ask Claude, GitHub issue submenu).

Discovered `scripts/install.sh` had its own hardcoded base64-encoded minimal config that bypassed `CONFIG_TEMPLATE` entirely on fresh installs. Regenerated the base64 from the Rust source via Python. Also: `migrate-config.sh` was passing `"[ai]"` as a required section — since `[ai]` is now intentionally commented out, it would append an empty `[ai]` stub on every existing config. Removed `[ai]` from the migration list.

**Breaks if:** Fresh `plexi-pr-N app` install produces a sparse config with only `[ai]` or similar instead of the full documented template; or if an existing config gets a stray empty `[ai]` stub appended on upgrade.

## 2026-05-11 — [GOTCHA] SDK key normalization (#1060) silently broke example apps using capitalized key names (PR #1071 → alpha)

PR #1060 added `_KEY_ALIASES` normalization in `_app.py`: `"Enter"→"return"`, `"Escape"→"escape"`, `"Backspace"→"backspace"`. Any app checking `key == "Enter"` (capitalized) silently stops responding to that key — no error, no warning. The stale alpha binary (pre-#1060) masked this during initial testing because the test was done at 02:24 and #1060 merged at 02:33 the same day.

**What NOT to do next time:** Don't validate key-event behavior against an alpha binary that was installed before a key normalization PR landed. Always check `_app.py:_KEY_ALIASES` for the canonical key name before writing any `key ==` check in an app.

**Breaks if:** Enter/Escape/Backspace stop responding in csv_viewer or backlog (navigation j/k still works; only the affected keys are silent).

## 2026-05-11 — [CHANGED] Migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui components (PR #1061 → alpha)
Added `SelectList` (stateful scrollable list with j/k, hit detection, scrollbar) and `FormField` (label + TextInput wrapper with `.submitted` property) to `sdk/python/plexi_sdk/ui.py`. Migrated both example renderers to use the declarative component system — no more manual `ctx.rect`/`ctx.text` layout math in the renderers.

Key non-obvious decisions:
- `Column(padding_top=0)` required whenever AppBar is first child — default `padding_top=SPACE_SM` shifts content 8px and breaks pre-computed hit regions.
- Form views use hybrid rendering (direct `.render()/.measure()` calls rather than `ctx.render(Column([...]))`) because they need `_hits` tracking for back/run click detection. Must call `ctx.clear(BG)` explicitly before form render since `ctx.render()` does it but direct calls do not. `ctx.clear()` defaults to `#000000` (black) not `BG` — must pass `BG` explicitly.
- `SelectList._total_content_h()` must not add `SPACE_XS` after the last item — causes over-clamping of scroll offset.
- Result view uses `Scrollable(Column([Label(...) for line in lines]))` rebuilt each frame, NOT `ScrollLog` — ScrollLog is not a scrollable component, j/k doesn't work on it.

**Breaks if:** mcp-renderer or descriptor-renderer show empty/black form view instead of the component UI, or j/k doesn't scroll the list, or the result view in mcp-renderer can't be scrolled with j/k.

## 2026-05-11 — [GOTCHA] PR builds load stable apps when focused pane cwd is ~/
`resolve_workspace_root` checks for `.plexi/` dir before the home-dir stop guard. Since `~/.plexi/` exists (stable profile), any terminal at `~/` or a subdir causes the AppRegistry to treat `~/.plexi/apps/` as local workspace apps, overriding the PR build's `~/.plexi-pr-<N>/apps/`. Manifests as PR build launching stable app code.

**Workaround:** cd focused terminal to `/tmp/` before opening apps in a PR build. Filed as issue #1064.


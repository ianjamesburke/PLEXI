# DEV_LOG

Development log for PLEXI. Tracks root causes, non-obvious decisions, abandoned approaches, and environment quirks that git history won't capture. Entries are newest-first — most recent at the top.

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


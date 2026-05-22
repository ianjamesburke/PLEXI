# DEV_LOG

Development log for PLEXI. Tracks root causes, non-obvious decisions, abandoned approaches, and environment quirks that git history won't capture. Entries are newest-first — most recent at the top.

## 2026-05-22 — [FIX] QuickNote Cmd+V paste falls through to terminal when zoom overlay active (PR #1658 → alpha)

When QuickNote is open and the zoom overlay is also visible, `TerminalView::set_focus(true)` is called unconditionally inside the CentralPanel render pass (`app/mod.rs:3999`). This `request_focus()` on the zoom terminal runs before the re-focus block at line 4312 reclaims focus for `quick_note_text`. On those frames, egui's TextEdit skips paste processing because `mem.has_focus(id)` is false — so `egui::Event::Paste` falls through to the terminal.

Fix: explicit paste handler in `draw_quick_note_modal` consuming `egui::Event::Paste` via `ctx.input_mut`, cursor-aware insertion via `TextEdit::load_state` + `CCursorRange`, `push_str` fallback. Same pattern as the Esc/Enter handlers — guaranteed to fire regardless of egui focus state on any given frame.

**Breaks if:** Cmd+V with QuickNote open sends text to the terminal instead of the note input.

## 2026-05-22 — [FIX] Sync NSWindow appearance with Plexi theme to fix black title in light mode (PR #1656 → alpha)

`with_titlebar_shown(false)` makes the title bar transparent but the OS still renders native title text using the window's `NSAppearance`. In system light mode, `NSWindow` defaults to light appearance, rendering the title text black — visible against Plexi's dark content showing through.

Fix: send `ViewportCommand::SetTheme(Dark/Light)` after `setup_style()` on startup and config hot-reload. This calls winit's `window.set_theme()` → `[window setAppearance: NSAppearanceNameDarkAqua]`, forcing white title text (hidden against dark background).

**Breaks if:** Title text appears black/visible in the titlebar area when macOS system is set to light mode.

## 2026-05-22 — [FIX] FocusLayer sync methods switched from pop to retain (PR #1655 → alpha)

Seven `sync_*_focus()` methods used `pop_focus_layer()` to remove their layer on close. `pop_focus_layer` only removes the top entry — if another layer was pushed above the target before its source state cleared, the buried entry survived and could later regain keyboard ownership after the top layer was dismissed.

Fixed by replacing `pop_focus_layer` with `focus_stack.retain(|l| *l != <layer>)` in all seven methods. `sync_cli_setup_prompt_focus`, `sync_context_inspector_focus`, and `sync_capability_modal_focus` already used `retain` — this brings the remaining methods into alignment.

**Breaks if:** Closing an overlay (command palette, rename pane, close confirmation, etc.) leaves keyboard input blocked in the terminal — the old buried layer entry regained ownership.

## 2026-05-21 — [FIX] CapabilityModal promoted to FocusLayer (PR #1621 → alpha)

Pressing Escape (or any key) inside the capability consent modal had no effect. Root cause: `show_prompt_modal` was called in step 5 (pane render), after `dispatch_app_key_events` (step 4) had already consumed any key — the modal never saw the keystroke.

Fix: add `FocusLayer::CapabilityModal` and render the modal in step 2 (early-modal path, before step 4). `sync_capability_modal_focus()` pushes/pops the layer based on whether the focused pane has pending prompts.

**Breaks if:** Pressing Escape inside the capability modal has no effect (app's key handler ate it first), or the modal fails to appear at all when a capability is requested.

**GOTCHA — egui_tiles restructures bare-pane roots on first render:** When a `Tree<PaneId>` has a bare `Tile::Pane` as root (no container wrapper), egui_tiles converts that tile ID to a `Tile::Container` on the first frame render, pushing the actual pane to a child tile. Any `TileId` stored before that first render (e.g. `focused_pane`) now points to a Container. Added `find_pane_in_tile()` — descends through any Container to reach the actual pane — and use it everywhere we resolve `focused_pane` to a `PaneId`. In tests, always run at least one idle frame before reading tile structure or setting focused_pane to a final value.

## 2026-05-19 — [FIX] Wikipedia key names and Bluesky capability consent (PR #1581 → alpha)

**Wikipedia (#1567):** SDK PR #677 added key normalization (`Enter`→`return`, `Escape`→`escape`, `Backspace`→`backspace`) in `_app.py` but wikipedia.py was never updated. All five key handler branches were silently dead. Fixed by renaming all three key strings to match SDK canonical names. Also added `capability_request("net.http")` in `on_init` (converted to async) — on fresh profiles the permission store withholds net.http until the user grants consent, so HTTP calls fail before ever being tried.

**Bluesky (#1569):** Same capability_request gap — `_fetch_discover` fired via `asyncio.create_task` before consent was prompted. Added `capability_request("net.http")` in `on_init` before creating the task. Also added explicit `data.get("error")` check in all three fetch paths: AT Proto API returns 4xx errors as valid JSON `{"error": "..."}` which json.loads() silently accepted, resulting in empty feed instead of error state.

**Breaks if:** Opening either app on a fresh profile does not show a network consent dialog; or Wikipedia Enter/Backspace/Escape keys don't respond after consent is granted.

## 2026-05-13 — [CHANGED] Self-documenting SDK — AST-based doc generator (PR #1258 → alpha)

Added `website/scripts/generate-sdk-docs.py` that parses SDK Python source via `ast` module and generates 5 Astro markdown pages (overview, App, RenderContext, Emitter, Widgets). Wired into `npm run build` so docs regenerate on every deploy. Docker build context moved from `website/` to repo root so Dockerfile can access both `website/` and `sdk/`.

Process revealed several SDK-side inconsistencies (filed as #1260): stale handler overview listing 12 of 26+ handlers, notify priority signatures lying to type checkers (`int | None = None` but raises TypeError on None), spawn handler naming breaking underscore convention, three dataclasses with zero docstrings. Generator also needed to filter `@property` getters/setters and strip rST `::` artifacts.

**Breaks if:** `npm run build` in `website/` fails (generator must run before Astro); or Railway deploy can't find SDK source (build context must be repo root, `SDK_SOURCE=/sdk/plexi_sdk`). See GOTCHAS.md `[railway]` entry for the three Railway dashboard settings that must align.

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


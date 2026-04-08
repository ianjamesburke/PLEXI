# Infinite Canvas Zoom Overhaul — PLEXI

## Context

PLEXI currently treats "contexts" as flat, indexed workspaces you switch between with Cmd+1-9. "Zoom" is just a fullscreen overlay toggle for one pane. The goal is to replace this with a spatial canvas where every screen lives at a 2D grid position, you navigate between screens by panning, splits are recursive (split → zoom in → split again), and zooming out gives progressively degraded previews until text is replaced by placeholders.

Working directory: `~/Documents/GitHub/PLEXI-dev/` (dev worktree, `dev` branch)

### Relationship to the broader Plexi vision

This plan covers **v1: spatial canvas zoom** only. The full vision (tracked in `~/Documents/GitHub/labs/active/plexi.md`) includes v2 features that build on this foundation:
- **SOB integration** — persistent attention bar + capture hotkey overlay (see `~/Documents/GitHub/labs/inbox/sob-system.md`)
- **Ephemeral launcher** — floating temporary shell overlay with fuzzy-find
- **Graphical canvas panes** — split panes with linked graphical viewers (file browser, image gallery)
- **Daemon terminals** — persistent background sessions that survive app close
- **Persistent SSH** — auto-reconnect remote panes
- **Agent management canvas** — live agent nodes with visible I/O
- **Living graph edges** — interactive connections between panes/agents/files

### Zoom model: discrete vs. continuous

**Decision needed.** The labs vision doc specifies continuous analog zoom (like Figma). This implementation plan uses discrete `ZoomLevel` enum (Normal → ZoomedOut1 → ZoomedOut2 → Overview) as a pragmatic first pass. 

**Recommendation:** Ship discrete first (simpler, testable, no animation complexity), then migrate to continuous zoom by replacing the enum with a `f32` scale factor and smooth `egui::Context::animate_value_with_time` transitions. The render thresholds (6px, 3px) work identically with either model — they key off effective font size, not zoom level.

### Named Frames

The labs vision doc defines **named frames** as the primary navigation primitive: `(x, y, zoom_level, label)`, Harpoon-style, max 9, bound to `<leader>1`–`<leader>9`. This replaces `Cmd+1-9` context switching entirely. The implementation plan's "Stage 5: Screen naming" and "Cmd+1-9 as screen bookmarks (later)" should converge on this — named frames are the bookmarks, not screens.

## Data Model

### Canvas replaces the context list

```rust
// PlexiApp.contexts: Vec<Context>  →  PlexiApp.canvas: Canvas

struct Canvas {
    screens: HashMap<ScreenPos, Screen>,
    camera: Camera,
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
struct ScreenPos { x: i32, y: i32 }       // grid coordinate

struct Camera {
    position: ScreenPos,             // which screen is centered
    zoom_level: ZoomLevel,           // Normal | ZoomedOut1 | ZoomedOut2 | Overview
}

#[derive(Clone, Copy, PartialEq)]
enum ZoomLevel {
    Normal,      // one screen fills viewport
    ZoomedOut1,  // ~3x3 grid visible, reduced font
    ZoomedOut2,  // ~5x5 grid, tiny font
    Overview,    // all screens as placeholders
}
```

### Screen replaces Context

```rust
// Before:
// Context { name, path, tree, panes, focused_pane, zoomed_pane }

// After:
struct Screen {
    name: String,
    path: PathBuf,
    tree: Tree<PaneId>,
    panes: HashMap<PaneId, TerminalPane>,
    focused_pane: Option<TileId>,
    zoom_stack: Vec<TileId>,  // stack of zoomed-into tiles (enables recursive zoom)
}
```

`zoom_stack` enables recursive zoom: push a TileId when zooming into a split, pop when zooming out. `zoom_stack.last()` is the effective root for rendering. Empty stack = full screen view.

### Keep egui_tiles

The `Tree<PaneId>` tiling within each screen stays. Recursive zoom works by temporarily setting `tree.root = zoom_stack.last()` before calling `tree.ui()`, then restoring it. egui_tiles handles layout of the visible subtree naturally.

## Keyboard Mapping

| Shortcut | Action |
|----------|--------|
| `Cmd+HJKL` | Navigate panes within screen. At edge with no pane in that direction → cross to adjacent screen |
| `Cmd+Shift+J` | Zoom in — dive into focused split (push zoom_stack). If zoom_stack is empty and at overview, canvas zoom in one level |
| `Cmd+Shift+K` | Zoom out — pop zoom_stack. If stack already empty, canvas zoom out one level |
| `Cmd+D` / `Cmd+Shift+D` | Split within current zoom level (unchanged) |
| `Cmd+T` | New tab (unchanged) |
| `Cmd+N` | New screen to the right of current |
| `Cmd+Shift+N` | New screen below current |
| `Cmd+plus` / `Cmd+minus` | Font size (unchanged) |
| `Cmd+1-9` | Repurpose as screen bookmarks (later) or remove |

### Zoom in/out behavior detail

`Cmd+Shift+J` (ZoomIn):
1. If camera is at ZoomedOut/Overview level → zoom canvas in one level toward Normal
2. If camera is at Normal and focused pane has a parent container → push parent container TileId onto zoom_stack (the focused split fills the screen)
3. If already zoomed into a leaf pane with no children → no-op

`Cmd+Shift+K` (ZoomOut):
1. If zoom_stack is non-empty → pop it (step back up to the parent split view)
2. If zoom_stack is empty and camera is at Normal → zoom canvas out to ZoomedOut1
3. If at ZoomedOut1 → ZoomedOut2. If at ZoomedOut2 → Overview. If at Overview → no-op.

### Navigation edge-crossing

When `Cmd+HJKL` navigation finds no pane in the requested direction within the current screen (and zoom_stack is empty, meaning we're at the screen's root level):
1. Compute adjacent screen position: `camera.position + dir.delta()`
2. If a screen exists there, switch `camera.position` to it
3. Focus the nearest pane on the entry side (e.g., navigating Right enters the left-most pane of the right screen)

If zoom_stack is non-empty, navigation stays scoped to the zoomed subtree — it does not cross screen boundaries from inside a zoom level.

## Render Thresholds

Based on effective font size (`base_font * scale`):
- **>= 6px**: full terminal text rendering
- **3-6px**: tiny text (visual impression of content, not readable — "lego text")
- **< 3px**: placeholder — colored block with terminal icon, no text rendering

At `ZoomLevel::Normal`: one screen fills viewport, full text.
At `ZoomedOut1`: ~3x3 grid visible, center screen ~60% scale, neighbors visible with reduced font.
At `ZoomedOut2`: ~5x5 grid, most screens at placeholder.
At `Overview`: all screens as placeholders on a mini-map.

**Performance**: when a screen is at placeholder level, do NOT call `TerminalView::new` at all. Just paint the static placeholder. This avoids all terminal rendering overhead for off-screen/tiny screens.

## Implementation Stages

### Stage 1: Rename Context -> Screen, add Canvas wrapper

**Goal**: Pure refactor, identical behavior, one screen at (0,0).

Files:
- **New**: `src/canvas.rs` — `Canvas`, `Screen`, `ScreenPos`, `Camera`, `ZoomLevel` structs + `impl Canvas` with accessors like `active_screen()` / `active_screen_mut()`
- **Rename**: `src/context.rs` — rename `Context` -> `Screen`, add `zoom_stack: Vec<TileId>` (initially empty, unused)
- **`src/app.rs`** — replace `contexts: Vec<Context>` + `active_context` with `canvas: Canvas`. Add helper `fn active_screen(&self) -> &Screen` that delegates. Update all `self.contexts[self.active_context]` references to use `self.canvas.active_screen_mut()`.
- **`src/pane_ops.rs`** — update all context references to use `self.canvas.active_screen_mut()`
- **`src/workspace.rs`** — update `SavedContext` -> `SavedScreen` with `pos: ScreenPos`
- **`src/sidebar.rs`** — update to iterate `canvas.screens`
- **`src/tiling.rs`** — no change

Verify: build and run. Behavior should be identical to current.

### Stage 2: Recursive zoom stack

**Goal**: `Cmd+Shift+J` zooms into a split's subtree. `Cmd+Shift+K` pops back out. Replaces the current fullscreen overlay.

Files:
- **`src/app.rs`** — replace zoom overlay logic (the scrim + inset rect approach) with zoom_stack root-swapping. Before `tree.ui()`, save `let original_root = screen.tree.root`, set `screen.tree.root = screen.zoom_stack.last().copied().or(original_root)`, call `tree.ui()`, restore `screen.tree.root = original_root`.
- **`src/keys.rs`** — replace `ToggleZoom` with `ZoomIn` + `ZoomOut`. Map `Cmd+Shift+J` -> `ZoomIn`, `Cmd+Shift+K` -> `ZoomOut`.
- **`src/pane_ops.rs`** — implement `zoom_in()`: find the parent container of the focused pane, push it onto zoom_stack. Implement `zoom_out()`: pop zoom_stack.
- **`src/tiling.rs`** — remove `zoomed_pane` field from `PlexiBehavior`. Remove the dark-placeholder and focus-guard logic for zoomed state. The zoom behavior is now handled by root-swapping, so tiling.rs just renders normally.
- **`src/context.rs`** — update `find_pane_in_direction_from` to accept an optional effective_root parameter. When zoom_stack is non-empty, only consider tiles that are descendants of the effective root.

Key detail: when zoomed into a subtree via zoom_stack, splitting creates new panes within that subtree. Closing panes within the subtree should pop the zoom_stack if the zoomed tile gets simplified away.

Verify: split (Cmd+D), zoom in (Cmd+Shift+J) -> one half fills screen. Split again, zoom in -> quarter fills screen. Cmd+Shift+K pops back each level.

### Stage 3: Cross-screen navigation + new screen creation

**Goal**: `Cmd+HJKL` crosses screen boundaries. `Cmd+N` creates adjacent screens.

Files:
- **`src/pane_ops.rs`** — update `navigate()`:
  1. Try `find_pane_in_direction_from` within current screen (respecting zoom_stack scope)
  2. If None AND zoom_stack is empty, compute `new_pos = camera.position + dir.delta()`
  3. If `canvas.screens.contains_key(&new_pos)`, set `camera.position = new_pos` and focus the entry-side pane
- **`src/pane_ops.rs`** — add `new_screen(dir: Direction)`: compute position at `camera.position + dir.delta()`, create new Screen there with one pane, navigate to it
- **`src/keys.rs`** — remap `Cmd+N` (currently `NewContext`) to `NewScreen(Right)`, add `Cmd+Shift+N` -> `NewScreen(Down)`. (Can also add `Cmd+Shift+H/L` for left/right if desired.)
- **`src/canvas.rs`** — add helper methods: `adjacent_pos(pos, dir) -> ScreenPos`, `has_screen(pos) -> bool`, `create_screen(pos, screen)`, `find_entry_pane(screen, from_dir) -> Option<TileId>`

Verify: Cmd+N creates a screen to the right. Cmd+L at the rightmost pane jumps to it. Cmd+H jumps back.

### Stage 4: Zoomed-out canvas rendering

**Goal**: `Cmd+Shift+K` (when zoom_stack is empty) zooms canvas out to see multiple screens. `Cmd+Shift+J` (at overview levels) zooms canvas back in.

Files:
- **`src/app.rs`** — in the main render path: when `camera.zoom_level != Normal`, instead of rendering one screen via `tree.ui()`, compute the visible screen grid and render each screen into an allocated rect at the appropriate scale.
- **`src/canvas.rs`** — add `fn visible_screen_layout(&self, viewport: Rect) -> Vec<(ScreenPos, Rect, f32)>` that returns (position, layout rect, scale factor) for each visible screen at the current zoom level.
- **`src/tiling.rs`** — add `font_scale: f32` field to `PlexiBehavior`. In `pane_ui()`, multiply the pane's font_size by font_scale when creating the `TerminalView`. Add `fn render_placeholder(ui: &mut Ui, rect: Rect, name: &str, accent_color: Color32)` for below-threshold screens.
- **`src/keys.rs`** — the existing `ZoomIn`/`ZoomOut` actions now also handle canvas zoom levels. Logic in app.rs dispatch:
  - `ZoomOut`: if zoom_stack non-empty, pop. Else, increment zoom_level (Normal -> ZoomedOut1 -> ZoomedOut2 -> Overview).
  - `ZoomIn`: if zoom_level > Normal, decrement zoom_level. Else, push onto zoom_stack.

Rendering approach for each visible screen at scale:
```rust
for (pos, rect, scale) in canvas.visible_screen_layout(viewport) {
    let screen = &canvas.screens[&pos];
    let effective_font = default_font_size * scale;
    if effective_font < 3.0 {
        render_placeholder(ui, rect, &screen.name, accent_color);
    } else {
        // Create child UI constrained to rect
        // Render screen's tree.ui() with font_scale applied
    }
}
```

Verify: from Normal with 2+ screens, Cmd+Shift+K shows them side by side at reduced font. Again -> smaller. Again -> placeholders. Cmd+Shift+J zooms back in.

### Stage 5: Polish + Named Frames

- Smooth animated transitions when crossing screens (`egui::Context::animate_value_with_time` for viewport offset)
- **Named Frames system**: replace `Cmd+1-9` context switching with frame bookmarks. Each frame = `(x, y, zoom_level, label)`. `<leader>a` marks current position, `<leader>1-9` jumps, `<leader>m` fuzzy picker. Max 9 frames. Persisted with canvas state.
- **Minimap** on left sidebar: shows all screen positions as blocks with named frame labels as anchors. Click to jump. Current screen highlighted.
- Edge indicators (subtle arrows at screen edges showing adjacent screens exist)
- Screen naming (auto-inherit from first pane's cwd basename)
- Persistence of full canvas state (screen positions, zoom_stack per screen, camera state, named frames)
- Visual accent on the "current" screen when zoomed out (brighter border)

### Stage 6: Continuous Zoom Migration (v1.1)

- Replace `enum ZoomLevel` with `camera_scale: f32` (1.0 = Normal, 0.3 = ~ZoomedOut1, etc.)
- Animate scale changes with `egui::Context::animate_value_with_time`
- Render thresholds stay the same (keyed off effective font size, not zoom level)
- Named frames store exact scale value instead of enum variant
- Scroll wheel zoom (Cmd+scroll) for analog control

### Future: v2 Features (builds on this foundation)

These are tracked in the labs active doc and depend on the canvas/zoom system shipping first:
- SOB attention bar (egui panel, ~60-80px above canvas)
- Ephemeral launcher overlay (centered floating pane, fuzzy-find, auto-dismiss)
- Graphical canvas panes (split pane: graphical viewer top, terminal bottom, synced to cwd)
- Daemon terminal mode (green glow border, persistent via tmux/mprocs backend)
- Persistent SSH panes (auto-reconnect, dim on disconnect)
- Agent management canvas (agent nodes with live I/O, visual workflow builder)
- Living graph edges (interactive connections between canvas nodes)

## Critical Files

| File | Changes |
|------|---------|
| `src/canvas.rs` | **New** — ~200 lines |
| `src/app.rs` | Heavy — replace context management, add canvas rendering |
| `src/context.rs` | Moderate — rename to Screen, add zoom_stack |
| `src/pane_ops.rs` | Moderate — cross-screen nav, zoom_in/zoom_out |
| `src/keys.rs` | Light — new actions, remapped shortcuts |
| `src/tiling.rs` | Light — remove old zoom overlay, add font_scale |
| `src/workspace.rs` | Light — update serialization structs |
| `src/sidebar.rs` | Light — screen grid instead of context list |

## Risks & Mitigations

1. **Root-swapping in egui_tiles**: Need to verify `tree.root` can be temporarily changed without side effects (like `simplify()` removing "orphaned" ancestors). Mitigation: only swap root for the duration of `tree.ui()`, restore immediately after. Avoid calling `simplify()` while root is swapped.

2. **PTY resize on zoom**: Zooming in/out changes the visible area. Decision: do NOT resize PTYs on zoom — treat it as viewport change only. Only resize on actual window resize. Terminals keep their grid dimensions regardless of zoom level.

3. **Font scaling in egui_term**: The custom terminal renderer may not handle arbitrary font sizes cleanly at very small scales. Mitigation: at < 6px effective size, skip terminal rendering entirely and use placeholders. Between 6-14px, the renderer should handle it fine since it already supports per-pane font sizing from 8-32px.

4. **Coupled state (from CLAUDE.md lesson)**: The `zoom_stack` is coupled to `focused_pane` and `tree` structure. When closing panes, splitting, or simplifying the tree, verify that zoom_stack entries still point to valid TileIds. Add validation: if `zoom_stack.last()` points to a tile that no longer exists, pop it.

## Verification Checklist

1. `cargo build` in PLEXI-dev — compiles cleanly
2. `cargo run` — launches, single screen works identically to current behavior
3. Recursive zoom: Cmd+D to split, Cmd+Shift+J to zoom in, Cmd+D to split again, Cmd+Shift+J to zoom deeper. Cmd+Shift+K pops back through each level.
4. Cross-screen nav: Cmd+N creates screen to the right. Cmd+L at rightmost pane jumps to it. Cmd+H jumps back.
5. Canvas zoom: with 2+ screens, Cmd+Shift+K (from Normal, stack empty) shows multi-screen view. Cmd+Shift+K again -> smaller. Cmd+Shift+J zooms back in.
6. Font size: Cmd+plus/minus still adjusts font size at all zoom levels.
7. Workspace persistence: quit and relaunch, canvas state (screen positions, splits) restored.

# Spatial Canvas — Design Exploration

> Research + design doc. **Not a ship proposal.** Scoping the "most spatial control over terminal panes anyone's ever had" idea into something actionable.

## Related existing plans

Two implementation plans already cover large parts of this space. This doc positions them rather than duplicating them.

- `docs/future-enhancements/infinite-canvas-zoom-implementation-plan.md` — 6-stage plan for `Canvas { screens, camera }`, recursive `zoom_stack`, discrete `ZoomLevel`, LOD thresholds, cross-screen `Cmd+HJKL`. Concrete file-by-file work.
- `docs/future-enhancements/spatial-group-splits-implementation-plan.md` — one-level spatial groups at canvas nodes; rejects tabs and recursive nesting for v1; defines Context/Node/Pane hierarchy.

The canvas-zoom plan is **the spine of this work**. The groups plan is a **local-density refinement** of what lives at one canvas cell. They are compatible and should ship in that order.

## 1. Vision

The user opens Plexi and sees a single pane — same as today. Pressing one chord zooms the camera out, revealing that this pane is one cell in a much larger 2D grid of contexts. Each neighboring cell is a different project, different shell, different agent session. At 30% scale, terminals still show "lego text" you can scan. At 5%, they're colored tiles on a minimap. Any tile can be clicked, any adjacent tile can be jumped to with `Cmd+HJKL` without leaving the keyboard. Inside a cell, `Cmd+Shift+J` zooms *into* a split, temporarily promoting a subtree to fill the screen. The camera, not the layout, is what moves — your workspace is a place, not a list.

**What the user can do that they can't today:** (a) see every running context at a glance without a modal sidebar, (b) navigate between projects by panning instead of `Cmd+1-9`, (c) zoom into a quarter of a split to focus work and zoom back out without re-arranging anything, (d) name spatial positions (frames) instead of numbered workspaces.

## 2. Prior art

| Name | What it does | Steal | Don't copy |
|---|---|---|---|
| **Niri** (Wayland compositor) | Scrollable workspace — columns of windows, infinite horizontal scroll, no tiling tree | Column-scroll navigation + named workspaces as spatial bookmarks | Pure 1D scroll — Plexi wants 2D |
| **PaperWM** (GNOME) | Same scrollable-column idea before Niri | Windows keep position across focus changes; no layout thrash | Single-monitor scroll axis |
| **tmux / Zellij** | Recursive binary-split trees per session | The `Tree<PaneId>` model Plexi already inherits via `egui_tiles` | Flat session list, no spatial relationship between sessions |
| **Warp** | Blocks, AI, cloud-synced sessions | Nothing spatial — Warp is anti-spatial, it's a notebook | Block-per-command model; spatial canvas is the opposite |
| **Excalidraw / tldraw** | Infinite 2D canvas, smooth continuous zoom, zoom-to-fit, zoom-to-selection | Continuous `TSTransform` camera; zoom-to-rect primitive; "level of detail" rendering at far zoom | Free-floating coordinates — Plexi needs grid discipline or the overview becomes noise |
| **Figma** | Same as tldraw plus performance at scale | Tile-based rendering budget; culling off-screen frames | Frame nesting — more hierarchy than a terminal app needs |
| **Obsidian Canvas** | Cards on an infinite plane, link edges | Edges-as-first-class-object (future: agent I/O graph) | Card metaphor — a terminal is not a card |
| **Blink Shell / Hyper** | Stylish terminals, some transparency | Transparent background stack — only if we commit to the 2.5D aesthetic | Zero layout innovation; single pane |
| **macOS Mission Control** | Zoom out to see all windows, click to enter | The "zoom as grand overview" gesture | It's a modal overlay, not the primary workspace |
| **Ghostty + Zellij** | Fast GPU terminal + multiplexer layered | Performance bar — if spatial rendering costs frame budget, the whole idea dies | Two-process layering — Plexi is one process |

**Two ideas worth stealing above all:** (1) Niri's *named spatial positions* as the replacement for `Cmd+1-9`; (2) Excalidraw's *continuous camera + LOD rendering* as the technical substrate.

## 3. Mental model candidates

1. **Niri-style column scroll + vertical stack.** Canvas is 2D but axes have meaning: columns = projects, rows = tasks within a project. Simple, linear, navigable with two key axes. *Example:* left column = "plexi", next column = "parallax", next = "personal". `Cmd+L` crosses columns.
2. **Free 2D grid (Excalidraw-ish but snapped).** Screens live at arbitrary `(x, y)` integer positions. No forced axis meaning. Matches the existing zoom plan's `ScreenPos { x: i32, y: i32 }`. *Example:* `Cmd+N` creates a screen to the right, `Cmd+Shift+N` below. The user builds their own shape.
3. **Fibonacci / golden-ratio deterministic split.** Every split uses a fixed ratio; "zoom to pane" is one click because all coordinates are predictable. Pretty, but purely aesthetic — it imposes layout discipline the user didn't ask for and conflicts with manual drag-resize.
4. **Fractal recursion — every pane is a Plexi canvas.** "Turtles all the way down." Technically elegant, conceptually impossible to explain, and the spatial-groups plan explicitly rejected it for v1 on overview-honesty grounds.
5. **Layered 2.5D — depth = zoom level, alpha = see-through.** At zoom-in you see a focused terminal; semi-transparent panels behind show the zoomed-out map. Visually arresting. Performance and legibility are both genuinely hard — stacking three alpha terminals is readable for no one.

**Recommendation:** #2 (free 2D grid), with #1's *named frames* layered on top as the navigation UX. #5's transparency is a stretch polish item, not a foundation. #3 and #4 go in the "wait and see" pile.

## 4. Zoom semantics — LOD for a terminal

A terminal at 10% zoom is illegible as text. "Zoom" must mean different things at different scales. Three levels of detail, keyed off **effective font size** (`base_font * scale`), matching the existing plan's thresholds:

| LOD | Effective font | What's rendered | Cost |
|---|---|---|---|
| **L0 — Live** | ≥ 6px | Full `TerminalView` against live PTY buffer; tab dots, cursor, colors | Full |
| **L1 — Lego text** | 3–6px | Same rendering path but `TerminalView` draws sub-pixel glyphs (visual impression of content, not readable) | Full path, smaller draws |
| **L2 — Placeholder** | < 3px | Solid rect with accent-colored border, screen name label, *no* `TerminalView` call at all | Near-zero |

L2 is the critical perf call. Hundreds of zoomed-out panes only work if they don't render terminals. A fourth layer — L3 minimap dots — falls out for free at `ZoomLevel::Overview`.

**Continuous vs discrete:** the existing plan proposes shipping discrete `ZoomLevel::{Normal, ZoomedOut1, ZoomedOut2, Overview}` first and migrating to continuous `f32` scale + `animate_value_with_time` in v1.1. This is correct — continuous zoom has animation bugs that only show up after you ship it, and LOD thresholds are the load-bearing thing, not the animation curve.

## 5. Core vs app decomposition

The load-bearing question. Plexi apps are subprocesses that emit `DrawCommand`s into a single pane's coordinate space (see `src/app_protocol.rs`). They cannot and should not reach out to sibling panes. Anything *spatial* is by definition core, but some overlays can be apps with a privileged capability.

| Concern | Core (Rust) | App (Python/Rust SDK) | Why |
|---|---|---|---|
| Canvas transform / camera state | **Core** | — | It's the window into every pane; lives in `PlexiApp` |
| Pane layout (`Tree<PaneId>`, splits, groups) | **Core** | — | `egui_tiles` is already core; apps can't mutate it |
| LOD rendering (L0/L1/L2) | **Core** | — | L2 skips `TerminalView` entirely — must be a renderer decision |
| Cross-pane navigation (`Cmd+HJKL` edge cross) | **Core** | — | Keybinding layer in `src/keys.rs` is core |
| Thumbnail / preview rasterization | **Core** | — | The renderer already draws these per-frame; exposing them to a subprocess is strictly worse |
| Minimap overlay widget | **Core** (simple), optional **app** (rich) | Possible | Simple version is an egui side panel. A richer "canvas inspector" app could receive a read-only canvas snapshot via a new `canvas.read` capability and draw its own view |
| Search-to-zoom ("jump to frame `foo`") | **Core** fuzzy-finder | — | Just a palette + camera target; no subprocess needed |
| Named frames storage | **Core** | — | Persisted with workspace state (see plan Stage 5) |

**One-line answer:** the spatial canvas is a **2-month core feature**, not a 2-week app. The app protocol is fine; nothing about spatial layout belongs on the other side of the JSON boundary.

## 6. Implementation phasing — three options

### Option A — Lean experiment (1–2 weeks)

**Scope:** Stage 1 + Stage 2 of the existing infinite-canvas plan. Rename `Context → Screen`, wrap in `Canvas { screens, camera }` with a single screen at `(0,0)`, replace the boolean `zoomed_pane` fullscreen overlay with a recursive `zoom_stack: Vec<TileId>` that root-swaps `tree.root` for rendering.

**Ships:** Recursive zoom-in/zoom-out *within* one screen. Split, zoom, split, zoom again, pop. No multi-screen yet.

**Enables:** Daily "focus mode" — zoom into a corner of a split to work on it, pop back out without disturbing layout.

**Explicitly not:** cross-screen navigation, minimap, thumbnails, camera scaling, named frames.

**Risk that kills it:** `egui_tiles` root-swapping has side effects during `simplify()`. Mitigation: only swap inside `tree.ui()`, never call `simplify()` while swapped. If root-swap doesn't work, fall back to per-screen sub-trees — still shippable in the same time.

### Option B — The "real" v1 (4–6 weeks)

**Scope:** Stages 1–4. Canvas with multiple screens, `Cmd+N`/`Cmd+Shift+N` to create adjacent screens, `Cmd+HJKL` edge-crossing, discrete `ZoomLevel` with 3 LOD tiers, renderer skips `TerminalView` at L2.

**Ships:** A daily-usable spatial multiplexer. `Cmd+Shift+K` zooms out to see 3×3 of contexts, `Cmd+Shift+J` zooms back in, pane navigation crosses screen boundaries at edges.

**Enables:** The vision's core claim — "zoom all the way out and get a top-down view of every context." Replaces `Cmd+1-9` context switching.

**Explicitly not:** continuous/animated zoom, transparency layers, graphical thumbnails (uses lego-text + placeholders, not rasterized previews), agent graph edges.

**Risk that kills it:** Font scaling in `egui_term`'s custom glyph renderer may degrade badly between 3–6px. Mitigation is already spec'd — use L2 placeholders aggressively. Second risk: PTY resize on zoom. Decision is already taken in the plan (*don't* resize PTYs on camera change).

### Option C — Full vision (2–3 months)

**Scope:** Stages 1–6 + named frames + minimap + spatial-groups plan layered in at Stage 2.5. Continuous `f32` camera scale, animated transitions, scroll-wheel zoom, named frame bookmarks, spatial groups inside a single canvas cell for tmux-like local density.

**Ships:** Everything the user described except transparency stacking and 3D perspective (both of which stay research, not shipped).

**Enables:** The pitch. "Most spatial control over terminal panes anyone's ever had."

**Explicitly not:** true 3D perspective (egui is 2D), semi-transparent pane stacking (legibility + perf are real), agent graph edges, daemon/SSH persistence. All tracked in the labs doc as v2.

**Risk that kills it:** Scope. Three months of core-rewrite time while Parallax is the killer-app in flight is a direct conflict. If Parallax needs API surface that the canvas refactor breaks, one of the two pauses.

## 7. Open questions

1. **Does `egui_tiles::Tree::root` actually tolerate temporary mutation?** If `simplify()` runs during a swapped frame, do orphaned ancestors get reaped? This is the single spike that would convert Option A from "probably" to "definitely."
2. **Does `egui_term`'s `TerminalView` render legibly in the 3–6px font range, or does antialiasing collapse it to mush?** Determines whether L1 "lego text" is a real tier or whether we jump L0 → L2.
3. **What does `Cmd+HJKL` do when the zoomed-into subtree has no pane in the requested direction?** Plan says "stay scoped"; user's mental model may expect "escape to parent zoom level." Needs a UX call before Stage 2 ships.
4. **Named frames vs screen positions — one concept or two?** The existing plan hedges. Cleanest: drop `Cmd+1-9` as screen shortcuts entirely; frames are the *only* jump primitive, and they store `(pos, zoom, label)`.
5. **Does the minimap live on the left sidebar (reuse `src/sidebar.rs`) or as a transient overlay (like command palette)?** Affects whether it's always-visible ambient context or a modal. Sidebar reuse is cheaper; overlay is the more "spatial" answer.

## 8. Recommendation

**Ship Option A now, in the background, as a single feature branch off `alpha`.** It's 1–2 weeks, it proves the root-swap and zoom-stack machinery against a real `egui_tiles` tree, and it delivers a visible daily win (recursive zoom-into-split) without touching cross-screen nav, LOD, or the camera model. Everything Option B needs is additive on top. Option B is the right *second* step once #125 (Claude Code backend swap) lands and Parallax has stabilized its core integrations — probably 4–6 weeks of core work in the Q3 window. Option C is a valid north star but not a plan; it's the labs vision doc. Don't schedule it. Let A prove the substrate, let B prove the daily utility, and let the user's actual usage patterns tell you whether C is even the right shape.

**Not Option B now:** it collides with Parallax for engineering attention and commits to LOD tuning before we know the `egui_tiles` root-swap works. **Not Option C now:** it's a 2–3 month refactor in a codebase that just shipped alpha and hasn't validated its app protocol under real users yet.

Proceed with Option A as a background-branch experiment?

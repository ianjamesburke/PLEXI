# Plexi Protocol — Media Primitives Proposal

**Status:** Proposal  
**Date:** 2026-04-15  
**Depends on:** `plexi-v2.1.md` (ui_primitives_v1)  
**Target version:** v2.2 (or a standalone `media_primitives_v1` feature flag)  
**Motivation apps:** Parallax Plexi App (clip editor + viewer), any future DAW/audio editor

---

## Why this proposal exists

Plexi v2.0 ships orchestration. v2.1 ships the UI primitive set that unblocks viewers, editors, forms, and dashboards. **Both specs explicitly park media playback and waveform rendering as v3+.** This proposal re-scopes them as v2.2-eligible by arguing they're a small, well-bounded addition — not a full media engine rewrite.

The concrete blocker: the Parallax Plexi App (spec at `parallax CLI/docs/parallax-plexi-app-spec-v2.3.md`) needs a video/audio player and waveform display to replace the "run CLI → open Finder → QuickTime" loop. A future DAW app (or an audio editor) needs the same waveform primitive plus playback position. These two apps cover the entire use case. Nothing here is speculative.

The v2.1 spec lists `video-editor` as needing "frame-accurate seek, waveform rendering" and parks it at v3+. This proposal is **not** a video editor. It's a viewer + waveform display. No frame-accurate trimming. No timeline scrubbing. No multi-track mixing. The distinction matters — a viewer is ~10× simpler to implement.

---

## Audit of what already exists

Before listing gaps, what does the current SDK already provide:

| Capability | Current state | Notes |
|---|---|---|
| Still image display | `ctx.image(path, x, y, w, h)` | Works. Used for stills grid. |
| Video thumbnail | `ctx.video_thumbnail(path, x, y, w, h)` | Works. Extracts first frame, caches in `~/.cache/plexi/thumbnails/`. Clicking opens with system default player. |
| File grid with auto-thumbnails | `ctx.file_grid(...)` | Works. Mixes images and video thumbnails. |
| External file drop target | `ctx.drop_target(id, x, y, w, h, accept)` | Works. Accepts files dragged from Finder. |
| Filesystem watching | Not a draw primitive — apps poll or use `watchdog`. | Out of scope for protocol. |
| Drag-to-reorder inside a list | **Missing.** | `ctx.scrollable_list` and `ctx.list` have no reorder support. |
| Inline horizontal slider | **Missing.** | No `ctx.slider` primitive exists. |
| Video playback (inline) | **Missing.** | `video_thumbnail` clicks out to system player. |
| Audio playback | **Missing.** | No audio primitive at all. |
| Waveform rendering | **Missing.** | No waveform primitive. |
| Playback position / scrub bar | **Missing.** | Depends on inline playback. |

---

## What this proposal adds

Four primitives. They share a feature flag: `media_primitives_v1`.

1. **`ctx.slider`** — horizontal or vertical value slider. Used for volume, playback position, timeline scrubbing.
2. **`ctx.sortable_list`** — a scrollable list with drag-to-reorder. Used for scene reordering in the Clip Editor.
3. **`ctx.media_player`** — inline video/audio player with transport controls. Used for the Viewer panel.
4. **`ctx.waveform`** — audio waveform display with playback cursor. Used for voiceover preview and a future DAW.

---

## 1. `ctx.slider`

The simplest primitive. A horizontal (or vertical) drag handle that returns a float value.

```python
ctx.slider(
    slider_id: str,
    value: float,           # 0.0–1.0 normalized, or use min/max
    on_change: Callable,    # fn(new_value: float)
    x: float, y: float,
    w: float, h: float = 8.0,
    min_val: float = 0.0,
    max_val: float = 1.0,
    vertical: bool = False,
    track_color: str = THEME.muted,
    fill_color: str = THEME.accent,
    handle_radius: float = 6.0,
    disabled: bool = False,
)
```

**Rendering:** filled track rect + unfilled remainder + circular handle at `value` position. Hover expands handle radius slightly (use `delta_time`). Active (dragging) state sets cursor to `ew-resize` (or `ns-resize` for vertical).

**Interaction:** `on_mouse_down` on the track area captures drag. Mouse move while captured updates `value` proportionally and calls `on_change`. Mouse up releases. Click anywhere on the track (not just the handle) jumps `value` to that position.

**Protocol:** Pure draw commands + the existing `on_mouse_down` / `on_mouse_move` / `on_mouse_up` event handlers. No new protocol commands needed. The SDK handles all interaction logic in Python. This primitive requires **no Rust changes** — it's a pure SDK addition.

**DAW use:** Volume fader per track, master gain, send levels. Exactly the same primitive, vertical orientation.

---

## 2. `ctx.sortable_list`

A scrollable list where rows can be dragged to reorder. The existing `ctx.scrollable_list` has no reorder support and its internals can't easily be extended — this is a new component.

```python
ctx.sortable_list(
    list_id: str,
    items: list[Any],
    render_item: Callable,  # fn(ctx, item, x, y, w, h, index, is_dragging)
    item_height: float,
    on_reorder: Callable,   # fn(from_index: int, to_index: int)
    x: float, y: float,
    w: float, h: float,
    selected: int = -1,
    on_select: Callable = None,
    gap: float = 2.0,
)
```

**Rendering:** Calls `render_item` for each visible item (virtualized — only renders items in the scroll viewport). The item under drag is rendered at cursor position + offset; a drop indicator line shows insertion point.

**Interaction:**
- Long-press (150ms) OR drag from a grip handle (a `⠿` icon the app can draw in `render_item`) initiates drag.
- Dragging snaps the ghost item to cursor; other items shift to show insertion position.
- Mouse up commits the reorder and calls `on_reorder(from_index, to_index)`.
- App owns the list data and mutates it in `on_reorder`. SDK doesn't own order state.

**Protocol changes needed:** One new event type: `DragReorder { list_id, from_index, to_index }`. Alternatively, the SDK can synthesize this from existing `on_mouse_down/move/up` events entirely in Python, avoiding a protocol change. **Recommended: Python-only implementation** — no new protocol event needed.

**Parallax use:** Scene list in Clip Editor. Drag row `scene_02` above `scene_01` → `on_reorder(1, 0)` → manifest YAML updated.

**DAW use:** Track list reordering.

---

## 3. `ctx.media_player`

An inline media player. Handles video and audio. Does not open an external application.

```python
ctx.media_player(
    player_id: str,
    path: str,              # absolute path to mp4/mov/mp3/wav/m4a
    x: float, y: float,
    w: float, h: float,
    playing: bool,          # app owns playback state
    position: float,        # seconds, app owns
    volume: float = 1.0,    # 0.0–1.0
    on_play: Callable = None,     # fn() — user pressed play
    on_pause: Callable = None,    # fn() — user pressed pause
    on_seek: Callable = None,     # fn(seconds: float) — user scrubbed
    on_ended: Callable = None,    # fn() — playback reached end
    show_controls: bool = True,   # draw transport bar
    loop: bool = False,
)
```

**Rendering:** Video frame at current `position` fills the rect (letterboxed). Below it, a transport bar: play/pause button, scrub slider (uses `ctx.slider` internally), current time / total duration, volume slider.

**Architecture — this is the hard one.** Video playback cannot be implemented in Python draw commands. The host must own the decoder. Two approaches:

**Option A — Host-native (recommended):** Add a `MediaPlayer { player_id, path, playing, position, volume }` draw command. The host renders the video frame directly using platform AV APIs (AVFoundation on macOS). App sends state each frame; host renders and sends back `MediaPlayerEvent { player_id, kind: "ended"|"time_update", position }` events. The app owns the playback state (playing/paused/position) and updates it from these events. This is ~200 lines in Rust (`AVPlayer` or `ffmpeg` decode loop, render to egui `ColorImage`).

**Option B — WebView bridge:** The SDK spawns an off-screen Electrobun/WKWebView that handles media, and Plexi composites its output as an image. Heavy, adds a WebView dependency, fragile. Rejected.

**Recommended: Option A.** New draw command + new event type. Feature flag: `media_primitives_v1`.

**Protocol additions for media_player:**

```rust
// Draw command
MediaPlayer {
    player_id: String,
    path: String,
    playing: bool,
    position_secs: f32,
    volume: f32,
    x: f32, y: f32, w: f32, h: f32,
    show_controls: bool,
    loop_: bool,
},

// Event (host → app)
MediaPlayerEvent {
    player_id: String,
    kind: String,       // "time_update" | "ended" | "error" | "loaded"
    position_secs: f32,
    duration_secs: f32,
    error: Option<String>,
},
```

**Implementation:** On macOS, use `AVPlayer` via `objc2` crate. Decode frames to `Arc<ColorImage>`, blit into egui painter. Transport bar is drawn on top. ~300 lines in `process_app.rs` + a new `media_player.rs` module.

**Side-by-side compare (Parallax Viewer):** App renders two `media_player` components with synchronized `position` state. When either sends a `time_update` event, the app updates both positions. This is pure app logic — no special protocol support needed.

---

## 4. `ctx.waveform`

Renders an audio waveform image with a playback cursor overlaid. Used standalone (for audio preview) or alongside `ctx.media_player` (for captioned video with audio track visualization).

```python
ctx.waveform(
    waveform_id: str,
    path: str,              # audio file path
    x: float, y: float,
    w: float, h: float,
    position: float = 0.0,  # playback cursor position, seconds
    duration: float = 0.0,  # total duration (for cursor math; 0 = auto from file)
    on_seek: Callable = None,   # fn(seconds: float) — click on waveform seeks
    color: str = THEME.accent,
    cursor_color: str = THEME.fg,
    bg: str = THEME.panel,
)
```

**Rendering:** Waveform peaks drawn as vertical bars (RMS per time bucket). Cursor is a vertical line at `position / duration * w`. Click anywhere on the waveform calls `on_seek` with the corresponding time.

**Architecture:** Waveform peak data must be computed from the audio file. Options:
- **Python-side:** App computes peaks via `audioop` / `wave` stdlib, passes peak data as a list of floats to `ctx.waveform`. SDK draws them as `ctx.line` calls. No new protocol commands. Slow for large files (compute once and cache).
- **Host-side:** Add a `ComputeWaveform { path }` draw command → host returns `WaveformData { peaks: [f32] }` event. Host caches per-path. Faster, caches persist across app restarts.

**Recommended: host-side computation with caching.** The Python option requires the app to ship audio decoding (not stdlib). Host computes once, stores in `~/.cache/plexi/waveforms/<path_hash>.bin`. Cache invalidated by file mtime.

**Protocol additions for waveform:**

```rust
// Draw command
Waveform {
    waveform_id: String,
    path: String,
    x: f32, y: f32, w: f32, h: f32,
    position_secs: f32,
    duration_secs: f32,
    color: String,
    cursor_color: String,
    bg: String,
},

// Event (host → app, fired once after peaks computed)
WaveformReady {
    waveform_id: String,
    path: String,
    duration_secs: f32,
    peak_count: u32,    // informational
},
```

**DAW use:** This primitive is the core of any DAW track lane. Render N `ctx.waveform` components stacked vertically, each with its own playback cursor (or one shared cursor position). The `on_seek` callback handles click-to-seek. Volume and pan per track use `ctx.slider`. Drag-to-reorder tracks uses `ctx.sortable_list`. That's 80% of a simple DAW track view from four primitives.

---

## Summary — what requires Rust vs. what's SDK-only

| Primitive | Rust changes | SDK changes | Complexity |
|---|---|---|---|
| `ctx.slider` | None | ~60 lines | Low |
| `ctx.sortable_list` | None (uses existing mouse events) | ~150 lines | Medium |
| `ctx.media_player` | High — AVPlayer decode loop, new draw command + event type | ~100 lines | **High** |
| `ctx.waveform` | Medium — audio decoding, peak cache, new draw command + event type | ~60 lines | Medium |

`slider` and `sortable_list` can ship immediately (SDK-only, no feature flag negotiation needed, backward-compatible). `media_player` and `waveform` require host changes and should gate behind `media_primitives_v1` in `[app.protocol.requires]`.

---

## Ship order

**All four primitives ship together in v2.2** as the `media_primitives_v1` feature set, timed with the video editor / DAW work. No early SDK-only drops.

Rationale: `slider` and `sortable_list` are lightweight, but shipping them piecemeal creates a partial API that the Parallax app can't fully use until `media_player` and `waveform` land anyway. Better to design all four together, ship them as a coherent feature, and let the Parallax Clip Editor and any DAW app adopt the full set at once.

---

## Open questions

1. **AVFoundation vs. ffmpeg for media_player host impl?** AVFoundation is macOS-only but zero-dependency. ffmpeg is cross-platform but adds a large dep. Since Plexi is macOS-only today, AVFoundation is the right call. Revisit when WASM target is on the roadmap.
2. **Who owns the playback clock?** Proposal: the host drives `time_update` events at ~10fps; the app updates its `position` state from them. The app is always slightly behind real playback — acceptable for a viewer, not for a DAW. For the DAW case, the host should expose a higher-frequency event or a monotonic clock offset so the app can interpolate. Defer this to when a DAW app actually needs it.
3. **`ctx.sortable_list` grip handle vs. long-press?** Long-press is discoverable but feels wrong on a desktop. A visible `⠿` grip drawn by the app in `render_item` is better. The SDK should call `set_cursor("grab")` when hovering over items to hint draggability.
4. **Waveform cache invalidation:** Use file mtime. If the user replaces an audio file at the same path, the cache misses and recomputes. Fast enough.

---

## Cross-references

- `parallax CLI/docs/parallax-plexi-app-spec-v2.3.md` — primary consumer of all four primitives
- `plexi-v2.1.md` §9 — explicitly parks video-editor at v3+; this proposal is scoped to viewer/waveform, not editor
- `sdk/python/plexi_sdk.py` — target for slider, sortable_list additions
- `src/process_app.rs` — target for MediaPlayer + Waveform draw command handling
- `proposals/core-advanced-ui-sdk.md` — original draft that seeded v2.1; tabs/grid landed there

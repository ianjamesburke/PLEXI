# SpawnPane — Unified Spatial App Primitive

**Date:** 2026-05-03
**Issue:** #592
**Status:** Approved for implementation

---

## Problem

Plexi has three disconnected ways to open apps: `AppCommand::SpawnApp` (native apps only), `DrawCommand::SpawnPane` (#527, narrowly scoped), and hardcoded host actions (`Action::OpenQuickNote`, `Action::OpenFileBrowser`). No overlay layout exists. The CLI cannot open apps. Agents cannot hand off interactive work to the human and receive a structured result.

The cd-injection bug in the file browser (`CdRequest` writes raw bytes to the PTY) is a symptom of the same root problem: no clean protocol for apps to affect the spatial layout.

---

## Design

### The four Z-layers

Every app placement is one of four Z-layers:

| `layout` value | Z | Description |
|---|---|---|
| `"split_v"` / `"split_h"` / `"split_above"` / `"split_left"` | 1 | Tile placement in the ribbon |
| `"overlay_pane"` | 2 | Anchored to triggering pane; zooms with it; one per pane |
| `"overlay"` | 3 | Full-window centered modal; backdrop; Escape dismisses |
| `"background"` | 0 | Job pile; auto-close on exit 0; notify on failure |

Z1 is the existing tiling layer (#563 ribbon lives here). Z2/Z3 float above it. Z0 is offscreen.

### Protocol

```json
{
  "type": "spawn_pane",
  "type_id": "file-explorer",
  "layout": "overlay",
  "args": ["~/Desktop"],
  "pipe_id": "sel-abc123"
}
```

- `type_id` — app manifest id or `"terminal"`
- `layout` — one of the layout strings above
- `args` — passed as argv to the spawned app
- `pipe_id` — optional; spawned app sends one `PipeMessage` back on completion

Host responds: `PlexiEvent::PaneSpawned { pane_id }` or `PlexiEvent::PaneSpawnError { reason }`.

### Rich args convention

Apps accept standard flags as part of `args`. The host passes them through unchanged; apps consume what they recognise and ignore the rest.

- `--seek=2:35` — open video/audio at timestamp
- `--line=142` — open editor at line number
- `--pipe=<id>` — register pipe_id for reply

### plexi open CLI

`plexi open <type_id> [args...] [--layout=X]`

Sends `SpawnPane` JSON over `PLEXI_SOCKET`. Same socket as `plexi notify` (#295). Works from any terminal — including one not inside Plexi.

```bash
plexi open file-explorer ~/Desktop --layout=overlay
plexi open video-player ~/clip.mp4 --seek=2:35 --layout=split_above
plexi open quick-note --layout=overlay
```

### AI↔human handoff loop

The pipe-back mechanism lets AI spawn an interactive app for the human and receive a structured result:

```
AI: SpawnPane("file-explorer", layout="overlay", pipe_id="sel-123")
    → human navigates, presses Enter on a file
    → file-explorer: PipeMessage("sel-123", { "path": "..." })
AI: receives PipeMessage → SpawnPane("audio-player", args=[path])
```

The agent never navigates UI. It spawns and awaits. The file explorer, text editor, and any interactive app are all human-interaction surfaces that feed back to the agent via pipe.

### Overlay constraints

**overlay (Z3):** Full-window. One at a time globally (second request replaces). Backdrop dims all panes below. Escape dismisses.

**overlay_pane (Z2):** Anchored to the triggering pane. When the anchor pane zooms (Cmd+Enter), the overlay zooms with it — both expand to fill the window together. One per anchor pane — second request replaces or opens as tile. Multi-overlay tiling (e.g. two side-by-side overlays on one pane) is deferred.

### Background / pile wiring

`layout: "background"` sends the app to the job pile (#528) immediately:
- Exit 0 → auto-close silently
- `DrawCommand::Complete { success: false, message }` or non-zero exit → notification via #291

### Quick Note migration

`Action::OpenQuickNote` → `SpawnPane { type_id: "quick-note", layout: "overlay" }`. Hardcoded handler removed. Quick Note is the first showcase that overlay is general enough to replace all special-cased launcher shortcuts.

---

## CdRequest removal

The file browser's `AppCommand::CdRequest` (which injects `cd` bytes into the linked terminal PTY) is removed. The correct model: terminal CWD changes → file browser `sync_cwd()` follows (one-way observer). The browser never pushes changes to the terminal. An explicit `T` key opens a new terminal at the current browser path via `SpawnPane`.

---

## What's not in scope

- Multi-overlay tiling (two overlays side-by-side on one pane)
- Per-overlay positioning/sizing within the overlay layer
- File browser rebuilt as SDK app (long game — depends on `fs.list` capability)
- Text editor app
- Ribbon navigation (#563)

---

## Implementation order

1. `DrawCommand::SpawnPane` + `PlexiEvent::PaneSpawned/Error` in `app_protocol.rs`
2. `Capability::PanesSpawn` in `app_permissions.rs`
3. Host routing: `overlay` render layer (full-window, backdrop, Escape)
4. Host routing: `overlay_pane` render layer (pane-anchored, zoom coupling)
5. Host routing: `background` → job pile entry
6. `plexi open` socket command (extend #295 infrastructure)
7. Python SDK: `ctx.spawn_pane()`
8. Quick Note migration
9. POC: agent → file-explorer overlay → pipe-back

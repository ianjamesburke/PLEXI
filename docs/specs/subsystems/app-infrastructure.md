# Plexi App Infrastructure

**Status:** v1 — stable
**Last updated:** 2026-04-14

---

## Overview

Plexi is a terminal multiplexer that hosts visual "apps" inside terminal panes. An app is any external process that speaks the Plexi app protocol — a newline-delimited JSON dialogue over stdin/stdout. Apps coexist with the terminal in the same pane: the shell stays alive underneath while the app's surface is drawn on top. Toggling focus between the terminal surface and the app surface is a core interaction; closing the app returns the pane to its underlying terminal.

Two kinds of apps exist:

- **In-process apps** — Rust types that implement Plexi's internal `App` trait and are compiled into the binary. Used for built-ins (file browser, audio player, permissions manager, text editor, quick-note). Zero IPC overhead. Not covered by this spec.
- **Out-of-process apps** — External executables (any language) that communicate with Plexi via line-delimited JSON on stdin/stdout. All third-party apps fall in this category and are the subject of this specification. They are loaded dynamically from the apps install directory via `manifest.toml`.

This document is the authoritative contract for out-of-process apps. Every event, draw command, manifest field, env var, file path, and convention listed here matches the behavior of the shipping code at the date in the header. The Python SDK (`sdk/python/plexi_sdk.py`) and Rust SDK (`sdk/rust/src/lib.rs`) are thin wrappers on top of this protocol.

### Terminology

| Term | Meaning |
|---|---|
| **App** | An external process speaking the Plexi protocol. |
| **Host** | The Plexi process that spawned the app. |
| **Pane** | A rectangular region of the Plexi window hosting a terminal + optional app surface. |
| **Surface** | The visual output of either the terminal or the app inside a pane. |
| **Frame** | One fully committed batch of draw commands, terminated by `frame_done`. |
| **Manifest** | `manifest.toml` next to the app entry point declaring id, capabilities, and launch config. |
| **Install dir** | The per-app directory under `<config_dir>/apps/<id>/`. |

### Supported languages

Any language that can read and write newline-delimited JSON on stdin/stdout and produce an executable entry point can host a Plexi app. Two officially supported SDKs:

- **Python** — `sdk/python/plexi_sdk.py`, `sdk/python/plexi_sdk_advanced.py`. Zero runtime dependencies. The canonical SDK: all shipping example apps use it.
- **Rust** — `sdk/rust/src/lib.rs`. `serde`-backed trait implementation; compiles to a standalone binary.

Apps written against the raw protocol (no SDK) are fully supported as long as they conform to this document.

---

## Install layout

All apps live under a per-build config directory resolved at runtime from the running binary's name:

| Binary name contains | Config dir |
|---|---|
| `alpha` | `~/.plexi-alpha/` |
| `beta` | `~/.plexi-beta/` |
| anything else | `~/.plexi/` |

Inside the config dir, apps live under `apps/<id>/`. A minimal app install looks like this:

```
~/.plexi-alpha/apps/
  wikipedia/
    manifest.toml        # required
    wikipedia.py         # the entry point (required, must be executable)
    plexi_sdk.py         # vendored SDK (optional but standard)
    feedback.jsonl       # written by Plexi when submit_feedback is called
```

Rules that the registry enforces on load (`src/app_registry.rs`):

- Each subdirectory of `apps/` is a candidate app. A candidate is loaded iff it contains a parseable `manifest.toml` AND the file named by `entry` exists AND is executable (Unix `+x` bit set).
- Missing/broken candidates are logged as warnings and skipped; they do not crash Plexi.
- **Standalone apps.** An app must run with no global install dependencies other than its language runtime. The Python SDK, any templates, and any helper modules ship inside the app's own install directory — Plexi never injects a library path. This is a deliberate product invariant: users can delete, clone, and move apps without tracking down missing global deps.
- **Python on macOS GUI builds.** macOS `.app` bundles do not inherit the user shell PATH, so `#!/usr/bin/env python3` resolves to Apple's frozen system Python 3.9. For `.py` entry points, Plexi probes a fixed list of well-known Homebrew paths (`/opt/homebrew/bin/python3`, `/usr/local/bin/python3`, then versioned variants `3.13`→`3.10`) and the first interpreter that reports `sys.version_info >= (3, 10)` wins. If none are found it falls back to `python3` on PATH and emits a warning. Apps MUST be written for Python >= 3.10, and the first line of every app `.py` file should be `from __future__ import annotations` so `X | Y` union types are safe.

### Local app directories

In addition to the global `<config_dir>/apps/`, Plexi walks up from the working directory and collects every `.plexi/apps/` it finds along the way. Apps from closer ancestors override those from farther ancestors, and local apps override global apps with the same `id`. Both the installed-apps map and the extension-map are rewritten this way on each registry load.

### Environment variables passed to apps

Every spawned app receives the following environment variables (set at spawn time in `ProcessApp::launch`):

| Env var | Value | Purpose |
|---|---|---|
| `PLEXI_APP_ID` | The `app.id` from `manifest.toml`. | Lets an app identify itself to Plexi (notification source, feedback file, logs). |
| `PLEXI_APPS_DIR` | Absolute path to `<config_dir>/apps/`. | Lets an app locate its own install dir (`$PLEXI_APPS_DIR/$PLEXI_APP_ID/…`) without guessing the build flavor. |
| `PLEXI_LAUNCH_MODE` | `"standalone"` (user launched it from the palette / CLI) or `"spawned"` (another app spawned it via `DrawCommand::SpawnApp`). | Lets an app change its UI based on whether it was launched directly or as a child of another app. |
| `PLEXI_PARENT_PANE` | Numeric pane id of the parent that spawned this app. Only set when `PLEXI_LAUNCH_MODE=spawned`. | Lets a spawned child address typed-pipe channels back to its parent. |

All other environment variables are inherited from the Plexi process. Apps MUST NOT assume any additional variables.

### Working directory

Plexi spawns each app with a working directory chosen by the caller (usually the pane's cwd or the directory of a file the app is opening). File paths emitted by the app in draw commands — `Image.path`, `VideoThumbnail.path`, `FileGrid.path`, etc. — are resolved relative to this cwd if not absolute.

---

## Manifest schema (`manifest.toml`)

Every app is loaded from a TOML manifest. The parsing is done by `AppRegistry::load_app` (`src/app_registry.rs`). The schema is intentionally small and is parsed by `serde(default)` on every optional field — fields omitted take the defaults listed below.

```toml
[app]
id          = "wikipedia"                        # required
name        = "Wikipedia"                        # required
entry       = "wikipedia.py"                     # required — path inside the app dir
version     = "0.2.0"                            # optional, default ""
description = "Search and read Wikipedia inline" # optional, default ""

[app.capabilities]
file_types      = ["md"]        # optional, default []
keybinding      = "cmd+shift+w" # optional, default None (not yet wired)
terminal_write  = false         # optional, default false
filesystem      = "read_only"   # optional, default "read_only" ("none" | "read_only" | "read_write")
env_file_access = false         # optional, default false
network         = false         # optional, default false
secrets_write   = false         # optional, default false
mouse_tracking  = false         # optional, default false

[app.launch]                     # optional — launch & companion configuration
mode               = "fullscreen"   # optional, default "fullscreen" ("fullscreen" | "windowed" | "companion")
companion          = "none"         # optional, default "none" ("none" | "terminal")
companion_position = "bottom"       # optional, default "bottom" ("bottom" | "right")
companion_size     = 0.25           # optional, default 0.25 (0.0..1.0)
companion_cwd      = "{launch_dir}" # optional, default "{launch_dir}" — supports {launch_dir} template

[app.spawnable]                  # optional — composition policy for spawn_app
allow_callers   = ["*"]                          # optional, default ["*"] (any caller); otherwise list of app ids
default_layout  = { kind = "fill" }              # optional, default { kind = "fill" }
allow_lifecycle = ["cascade", "orphan", "prompt"] # optional, default all three
```

### `[app]` fields

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | Stable identifier used in logs, commands, file attribution. Must be unique within the apps directory. |
| `name` | string | yes | Human-readable display name shown in the app switcher and titles. |
| `entry` | string | yes | Path (relative to the app directory) to the executable. Resolved against `<app_dir>/<entry>`. Must be a file and must have the Unix executable bit set — the registry refuses to launch non-executable entries with a clear error. |
| `version` | string | no | Free-form semver. Surfaced in logs. |
| `description` | string | no | One-line description. |

### `[app.capabilities]` fields

Capabilities are declared at install time in the manifest and converted to runtime permissions by `AppCapabilities::to_permissions`. Manifest apps are always assigned the `sandboxed` trust level.

| Field | Type | Default | Semantics |
|---|---|---|---|
| `file_types` | string array | `[]` | File extensions this app can open. Registered in the extension→app map; first app to register an extension wins (local apps override global). |
| `keybinding` | string or null | `null` | Reserved for a global keybinding. Parsed but not yet wired into the shortcut engine. |
| `terminal_write` | bool | `false` | Allow the app to emit `run_in_terminal` / `cd` commands. |
| `filesystem` | `"none"` / `"read_only"` / `"read_write"` | `"read_only"` | Declared filesystem reach. Enforced by the app API layer on structured requests. |
| `env_file_access` | bool | `false` | Allow the app to read `.env` / credential files. |
| `network` | bool | `false` | Allow network requests. |
| `secrets_write` | bool | `false` | Allow writing secrets to the system keychain via the secrets API. |
| `mouse_tracking` | bool | `false` | Opt in to receiving continuous `mouse_move` events. Off by default to avoid flooding the pipe. Can also be toggled at runtime with the `mouse_tracking` draw command. |

### `[app.launch]` fields (optional)

When present, Plexi uses this table to decide how to place the app's pane on launch. In v1 this controls two things: the app's `mode` (how its pane occupies the slot it's dropped into) and, when `companion` is set to a non-`"none"` value, a companion split declared statically at launch time.

| Field | Type | Default | Semantics |
|---|---|---|---|
| `mode` | `"fullscreen"` / `"windowed"` / `"companion"` | `"fullscreen"` | How the app occupies its pane. `"fullscreen"` fills the slot; `"windowed"` is reserved for a future floating-window mode; `"companion"` triggers the companion-split below. The v1 host treats anything other than `"companion"` as fullscreen. |
| `companion` | `"none"` / `"terminal"` | `"none"` | What runs in the companion slot. `"none"` disables the auto-split; `"terminal"` keeps the v1 companion-terminal behavior. |
| `companion_position` | `"bottom"` / `"right"` | `"bottom"` | Split orientation when `companion != "none"`. |
| `companion_size` | float `0.0..1.0` | `0.25` | Fraction of the split the companion takes. |
| `companion_cwd` | string | `"{launch_dir}"` | Working directory for the companion. The literal `{launch_dir}` template expands to the pane's launch directory. |

### `[app.spawnable]` fields (optional)

Declares this app's composition policy — who may spawn it as a child via `DrawCommand::spawn_app`, what layout it defaults to when the caller doesn't specify one, and which lifecycles it accepts. A missing `[app.spawnable]` table means permissive defaults: any caller, any lifecycle, `fill` layout.

| Field | Type | Default | Semantics |
|---|---|---|---|
| `allow_callers` | string array | `["*"]` | List of `app.id`s permitted to spawn this app. `"*"` means any caller. If the spawning app's id is not in this list (and the list does not contain `"*"`), the spawn is refused and an error notification is sent back to the caller. |
| `default_layout` | layout object | `{ kind = "fill" }` | Layout applied if the caller omits `layout` on the spawn request. Same shape as the `layout` field on `spawn_app` (see below). |
| `allow_lifecycle` | string array | `["cascade", "orphan", "prompt"]` | Lifecycles this app accepts. Values: `"cascade"`, `"orphan"`, `"prompt"`. If the caller asks for a lifecycle not listed here, the spawn is refused. |

### `[app.layout]` fields (optional)

Declares pane-size hints that the SDK reads at app startup. Unlike the
other manifest tables, `[app.layout]` is consumed on the **app side** (via
`load_manifest` in the Python SDK and `load_manifest_layout()` in the Rust
SDK) rather than by `AppRegistry`. The host treats the table as opaque and
passes it through to the spawned app.

```toml
[app.layout]
min_width  = 400   # logical pixels, default 0 (no floor)
min_height = 200   # logical pixels, default 0 (no floor)
```

| Field | Type | Default | Semantics |
|---|---|---|---|
| `min_width` | int / float | `0` | Minimum pane width in logical pixels. When the pane is narrower than this on the current frame, the SDK draws a built-in "too small" fallback (background rect + centered `min size: W x H` label + directional arrow + dim `current: w x h` subtitle) and skips the app's render handler entirely. |
| `min_height` | int / float | `0` | Minimum pane height. Same semantics as `min_width` on the vertical axis. Either axis can be zero to mean "no floor on that axis". |

When a pane is below the floor on one axis only, the fallback arrow points
in that direction (`→` for width, `↓` for height). When below on both,
the arrow is `↘`.

Apps that need to compute their minimum size at runtime (for example, from
font metrics) can set it programmatically instead of in the manifest — see
[`sdk/python/README.md`](../../sdk/python/README.md) for `App.set_min_size`
and [`sdk/rust/README.md`](../../sdk/rust/README.md) for the
`App::min_size` trait method. Both SDK READMEs are the source of truth for
the breakpoint-dispatcher and fallback-rendering behavior; the host does
not currently do any structured parsing of this table.

### Breakpoint dispatchers (SDK feature)

Breakpoints are a declarative alternative to hand-rolling an
`if width < 400` branch inside a single render handler. Both SDKs expose a
way to register multiple render functions keyed on a `(min_width,
min_height)` pair; on every render event the SDK picks the most specific
match whose constraints fit the current pane (largest `min_width *
min_height` product where both axes still fit).

- **Python:** `@app.breakpoint(min_width=..., min_height=...)` stacks
  handlers; `@app.breakpoint()` is the `(0, 0)` fallback. Mutually
  exclusive with `@app.on_render`.
- **Rust:** `BreakpointSet::new().breakpoint(w, h, |ctx| ...)` for the
  stateless-closure form, or the `pick_breakpoint(width, height, &[...])`
  free function when the render methods need `&mut self`.

Both implementations live **on the SDK side** — the host does not know
about breakpoints. See the SDK READMEs for full code examples.

---

## Protocol overview

Plexi and the app exchange one JSON object per line on stdin/stdout. No framing, no length prefixes — every line is a complete JSON message. Each side ignores blank lines.

**Plexi → App:** messages of type `PlexiEvent` (see `src/app_protocol.rs :: PlexiEvent`). Tag: `"type"`, snake_case.
**App → Plexi:** messages of type `DrawCommand` (see `src/app_protocol.rs :: DrawCommand`). Tag: `"type"`, snake_case.

Apps SHOULD write each JSON line followed by a newline and flush stdout. The Python SDK sets `sys.stdout.reconfigure(line_buffering=True)` at startup and explicitly passes `flush=True` to every `print`. Failure to flush will cause frames to accumulate in the OS pipe buffer and visually freeze the app.

### Frame lifecycle

```
Plexi                                                 App
 │                                                     │
 │ ── spawn child process, pipe stdin/stdout/stderr ── │
 │                                                     │
 │ ── { "type": "init", width, height, ppp } ───────── │
 │                                                     │
 │ ── { "type": "render", width, height, delta } ───── │
 │                                                     │── draw_command ──►│
 │                                                     │── draw_command ──►│
 │                                                     │── frame_done ─────►│
 │ ◄── commits pending frame, paints                   │
 │                                                     │
 │ ── { "type": "key", key, modifiers } ─────────────  │
 │ ── { "type": "render", … } ──────────────────────── │
 │                                                     │ (draws updated state)
 │                                                     │
 │ ── { "type": "shutdown" } ────────────────────────── │
 │                            (app exits)              │
```

### Two-buffer rendering

Plexi maintains two internal frame buffers per `ProcessApp`:

- `frame` — the last fully committed frame. Always a complete, valid snapshot. This is what the painter reads from.
- `pending_frame` — accumulates draw commands as they arrive.

On receiving `frame_done`, Plexi atomically swaps `pending_frame` into `frame`. Partial frames are never painted — if the app panics or stalls in the middle of emitting commands, the previous frame is still visible. If multiple `frame_done`s arrive in a single drain cycle, the last complete frame wins.

This means apps can freely emit commands in any order between `frame_done`s, and Plexi will never tear.

### Render cadence

After each `render` event, the app is expected to flush a full frame (draw commands + `frame_done`). Plexi requests repaints at ~60 FPS (`request_repaint_after(16ms)` in `process_app.rs`). If the app is slow, Plexi will simply keep displaying the last committed frame and the next `render` event fires on the next repaint tick. There is no enforced timeout: a slow app degrades gracefully rather than being killed.

The `delta_time` field on `render` is the wall-clock time in seconds since the previous `render` was sent, measured by Plexi. Apps should use this for animation stepping instead of local timers so that animation stays time-linear across hot-reloads and frame drops.

---

## Protocol — Plexi → App (`PlexiEvent`)

Every event is a JSON object with a `"type"` discriminator. Unknown events MUST be ignored silently by apps; Plexi may add new event types in future minor releases (see "Versioning & stability").

| Event | Fields | When | Reply expected |
|---|---|---|---|
| `init` | `width: float`, `height: float`, `pixels_per_point: float` | Once, immediately after spawn. Do NOT render yet — wait for the first `render`. | None |
| `render` | `width: float`, `height: float`, `delta_time: float` | Every repaint tick (~60 FPS). | A batch of draw commands terminated by `frame_done`. |
| `resize` | `width: float`, `height: float` | Whenever the pane size changes by more than 1 logical pixel from the last sent size. | None (the next `render` carries the new size too). |
| `key` | `key: string`, `modifiers: { shift, ctrl, alt, cmd: bool }` | App surface has focus and the user pressed a key, OR the user typed a printable character. | None |
| `click` | `x: float`, `y: float`, `button: "primary" \| "secondary"` | Defined in the protocol for legacy reasons; the v1 host does not emit `click` — mouse input is delivered via `mouse_down` / `mouse_up` instead. Apps MAY handle it for forward compatibility but should not rely on it. | None |
| `mouse_down` | `x: float`, `y: float`, `button: "left" \| "right" \| "middle"` | User pressed a mouse button inside the app surface. | None |
| `mouse_up` | `x: float`, `y: float`, `button: "left" \| "right" \| "middle"` | User released a mouse button inside the app surface. | None |
| `mouse_move` | `x: float`, `y: float` | User moved the cursor inside the app surface. **Only sent when `mouse_tracking = true` in the manifest or after the app has emitted a `mouse_tracking` draw command with `enabled: true`.** | None |
| `scroll` | `x: float`, `y: float`, `delta_x: float`, `delta_y: float` | Scroll wheel / trackpad scroll over the app surface. `x, y` is the cursor position; `delta_x, delta_y` is smooth scroll in logical pixels. | None |
| `command` | `text: string` | User submitted a command via the Plexi command bar while the app had focus. | None; the app can respond by emitting `run_in_terminal`, state mutations, etc. |
| `drop` | `target_id: string`, `paths: string[]` | User dropped files from outside Plexi onto a `drop_target` the app declared in the last committed frame. `target_id` matches the `id` the app passed to `drop_target`. `paths` are absolute host paths already filtered against the target's `accept` list. | None |
| `get_state` | (none) | Plexi needs a state snapshot (undo, save, hot-reload handoff, Permissions Manager, etc.). | A `state` draw command with the four buckets. MUST reply exactly once. |
| `set_state` | `user_state: any`, `derived: any`, `session: any`, `persistent: any` | Plexi is restoring a previously captured state (undo/redo, hot-reload, external hydrate). | None; the app applies the buckets and the next `render` reflects the new state. |
| `shutdown` | (none) | The pane is closing or the app is being replaced. | App SHOULD exit its event loop promptly. Plexi will also send SIGKILL if the process has not exited when it is dropped. |

### Key event encoding

Key events come from two distinct egui sources, each with its own rules:

1. **Non-printable / control keys** (`Backspace`, `Enter`, `Tab`, `ArrowUp`, `ArrowDown`, `F1`..`F12`, `Escape`, `Home`, `End`, `PageUp`, `PageDown`, `Delete`, `Insert`, `Space`, etc.) — forwarded by serializing the egui `Key` enum via `format!("{key:?}")`. Values are egui's Rust `Debug` strings — e.g. `"Enter"`, `"Backspace"`, `"ArrowUp"`, `"F5"`, `"Escape"`, `"Space"`. Apps should treat these as stable string identifiers.
2. **Printable characters** (letters, digits, punctuation, Unicode) — forwarded as one `key` event per character, with the `key` field set to the single-character string the user typed (correct case, correct locale). Modifiers on these events are `{shift: false, ctrl: false, alt: false, cmd: false}` — the character already encodes shift state via casing.

There is one important interaction: **bare unmodified letter keypresses are NOT sent via the enum path.** egui fires both `Key::A` and `Text("a")` for a plain `a` keypress; forwarding both would cause apps to see every letter twice, so Plexi drops the enum variant when the letter has no modifiers. Modified letters (e.g. `Cmd+S`, `Ctrl+C`) are forwarded via the enum path because they do NOT also fire as `Text` events.

This means apps can reliably handle:

- `if key == "Enter": …` — for control keys.
- `if len(key) == 1 and key.isprintable(): …` — for user input, with correct case.
- `if key == "S" and mods["cmd"]: …` — for shortcuts. The enum serializer uses PascalCase for letters, so shortcut comparisons should use uppercase letter names.

### Modifiers

`modifiers` is always `{"shift": bool, "ctrl": bool, "alt": bool, "cmd": bool}`. On macOS, `cmd` is the ⌘ key; on Linux, it is the Super/Windows key. Apps should prefer `cmd` for platform-native shortcuts; use `ctrl` for terminal-style shortcuts only.

### Mouse button encoding

Mouse button strings are `"left"`, `"right"`, `"middle"`. Coordinates are logical pixels (post-DPI scale) measured from the app surface's top-left corner. The host subtracts the pane's inner frame margin (8 px) and clamps hit-testing to the pane rect before sending.

`mouse_move` is throttled to "off by default": the host only forwards move events when the app has opted in (via the manifest `mouse_tracking = true` or the runtime `mouse_tracking` draw command). Apps that draw hover states MUST opt in or their hover will feel dead.

### Scroll event encoding

`scroll` carries `delta_x` and `delta_y` in the same logical-pixel space as mouse coordinates. Plexi uses egui's `smooth_scroll_delta`, which provides momentum-integrated scroll (not discrete wheel clicks). A typical trackpad flick might produce deltas in the 1–50 range per frame.

### Drop event encoding

Drop targets are re-declared every frame by the app. When the user drags files from outside Plexi over the pane, Plexi finds the topmost `drop_target` in the last committed frame whose rect contains the cursor and displays a highlight. On release, Plexi filters `paths` against the target's `accept` extension list (empty list = accept anything; extensions matched case-insensitively with or without leading dots) and emits a single `drop` event with only the matching paths.

If the drop hits a target but the `accept` filter removes all paths, the drop is still considered consumed (the user's intent was clear) — but no `drop` event is sent. Apps should re-declare drop targets on every frame; skipping a frame temporarily disables the zone.

### Init vs Resize

`init` fires once immediately after spawn. `resize` fires on any subsequent change of the pane dimensions. Both carry the current pane `width` and `height`. The `render` event also carries `width` and `height` so apps that don't care about resize semantics can rely on `render` alone.

### Pixels per point

`init.pixels_per_point` is the logical-to-physical DPI scale factor. Apps should ignore this unless they are drawing images or need pixel-perfect grids; all other coordinates in the protocol are already in logical (dpi-independent) pixels.

---

## Protocol — App → Plexi (`DrawCommand`)

Draw commands are what the app sends to describe a frame (and out-of-band side effects). Every draw command is a JSON object with a `"type"` discriminator. Plexi ignores draw commands whose shape it can't parse (logged at warn level) — the rest of the frame still paints. Unknown command types are treated as parse errors.

Drawing commands use logical pixels with origin at the app surface's top-left. Colors are CSS-style hex strings (`#rrggbb` or `#rrggbbaa`). Unrecognized color strings fall back to theme defaults.

### Geometry primitives

| Type | Fields | Notes |
|---|---|---|
| `rect` | `x, y, w, h: f32`, `fill: string`, `radius: f32 = 0` | Filled rectangle with optional rounded corners. |
| `text` | `x, y: f32`, `text: string`, `size: f32`, `color: string`, `monospace: bool = false`, `bold: bool = false` | Single-line text at baseline-top position. `monospace = true` uses the terminal font so text lines up with PTY output. `bold` is reserved — egui does not currently render a bolder variant but the flag is parsed. |
| `line` | `x1, y1, x2, y2: f32`, `color: string`, `width: f32 = 1.0` | Straight-line segment. |

### Media primitives

| Type | Fields | Notes |
|---|---|---|
| `image` | `path: string`, `x, y, w, h: f32`, `fit: "contain" \| "cover" \| "fill" = "contain"`, `rounding: f32 = 0.0` | Loads and caches the file texture. `path` is absolute or resolved against the app's cwd. Cache key = absolute path + mtime. On decode error, Plexi paints a red error placeholder with an × glyph. |
| `video_thumbnail` | `path: string`, `x, y, w, h: f32`, `show_play_button: bool = true`, `timestamp_seconds: f32 = 0.0` | Extracts a frame at `timestamp_seconds` using `ffmpeg`, caches it in `~/.cache/plexi/thumbnails/`, overlays a centered play triangle (unless disabled), and makes the rect clickable — clicking opens the video with `open` on macOS. First render returns a loading placeholder; the thumbnail replaces it on the next repaint. |
| `file_grid` | `x, y, w, h: f32`, `path: string? + filter: string[]?`, `paths: string[]?`, `item_size: f32 = 96.0`, `columns: u32?`, `show_labels: bool = true` | Grid of file thumbnails. Exactly one of `path`/`paths` must be provided. `filter` accepts glob-ish patterns: `"*.png"`, bare extensions like `"png"`, or substring patterns. Images use the image cache; videos (mp4/mov/webm/mkv/m4v/avi) use the video thumbnail cache; everything else gets a generic icon with the extension label. Each item is clickable and opens with `open`. |

### Input regions

| Type | Fields | Notes |
|---|---|---|
| `drop_target` | `id: string`, `x, y, w, h: f32`, `accept: string[] = []`, `label: string?` | Declares a region that accepts dropped files from outside Plexi. Stateless — must be re-emitted every frame. `accept` is a list of extensions (lowercase, no dot); empty means accept anything. While the user is dragging files, Plexi draws a subtle accent highlight and the optional `label`. On drop, Plexi sends a `drop` event back with the matching `id`. |

### High-level widgets

| Type | Fields | Notes |
|---|---|---|
| `list` | `items: ListItem[]`, `selected: usize`, `item_height: f32 = 20.0` | Scrollable list rendered by Plexi at the pane origin with implicit full-pane layout. Handles its own scroll state. `ListItem = { label: string, secondary: string?, icon: string?, is_dir: bool = false }`. **Caveat — full-pane only.** This primitive has no x/y/w/h parameters: Plexi renders it at the app surface origin with full available width. It will overlap anything else in the frame and cannot be used inside a split layout. For positioned lists, render manually with `text` + `rect`. |

### Cursor & input control

| Type | Fields | Notes |
|---|---|---|
| `set_cursor` | `cursor: "default" \| "pointer" \| "grab" \| "grabbing" \| "crosshair" \| "text"` | Sets the cursor icon over the app pane. **Per-frame:** resets to `"default"` on each new frame, so apps must re-emit every frame they want a non-default cursor. Unknown values fall back to `"default"`. |
| `mouse_tracking` | `enabled: bool` | Toggles delivery of `mouse_move` events. **Stateful:** persists until changed. Off by default. Overrides the manifest capability for the life of the process. |

### Terminal side effects

These commands mutate the linked terminal rather than the draw buffer. They are picked up by Plexi and translated into `AppCommand`s at frame drain time, then dispatched to the correct terminal pane (same pane for `AppWithCompanion` apps; sibling pane for legacy split apps).

| Type | Fields | Notes |
|---|---|---|
| `run_in_terminal` | `command: string` | Executes a shell command in the linked terminal pane. Requires `terminal_write = true`. |
| `cd` | `path: string` | `cd`s the linked terminal pane to `path`. Requires `terminal_write = true`. |

### Logging & observability

| Type | Fields | Notes |
|---|---|---|
| `log` | `level: "error" \| "warn" \| "info" \| "debug"`, `message: string` | Forwarded to the Plexi logger with target `app::<app_id>`. Unknown levels default to `info`. |
| `notification` | `priority: u8`, `title: string`, `body: string?`, `source_app: string` | Appends a notification to Plexi's notification log, increments the status-bar unread count, and surfaces the notification in the Cmd+Shift+N palette. Priorities: `0` = info, `1` = normal, `2` = high, `3` = urgent — the MVP does not style by priority. If `source_app` is the empty string, Plexi substitutes the process app id. |
| `cost_report` | `app_id: string`, `service: string`, `model: string`, `input_tokens: u64`, `output_tokens: u64`, `cost_usd: f64`, `operation_id: string?`, `timestamp: string?` | Reports an LLM call's cost for attribution. Plexi accumulates a per-session total in memory and appends a JSON line to `<config_dir>/costs.jsonl`. If `timestamp` is omitted, Plexi substitutes `Utc::now()` as RFC 3339. |

### State management

| Type | Fields | Notes |
|---|---|---|
| `state` | `user_state: any`, `derived: any`, `session: any`, `persistent: any` | Response to a `get_state` event. MUST be emitted exactly once per `get_state`. All four buckets are required (use `{}` for empty). Plexi feeds this into the undo/redo state machine (see "State persistence"). |

### App composition

| Type | Fields | Notes |
|---|---|---|
| `spawn_app` | `app_id: string`, `args: string[] = []`, `parent: string = "self"`, `layout: object = { kind: "fill" }`, `lifecycle: "cascade" \| "orphan" \| "prompt" = "cascade"`, `linked: bool = true`, `wire_channels: string[] = []` | Ask Plexi to launch another app and place it in a layout slot relative to this pane. The foundation primitive for app composition — e.g. a file browser pressing Enter on a `.txt` emits `spawn_app("text-editor", args=[path], layout={kind:"cols",slot:1,ratio:0.5})` to open the editor in a 50/50 right split lifecycle-bonded to itself. See the **App Spawning** section below for the full contract. |

### Frame control

| Type | Fields | Notes |
|---|---|---|
| `frame_done` | (none) | Commits the pending frame. After receipt, Plexi atomically swaps `pending_frame` into `frame` and clears `pending_frame`. Everything between two `frame_done`s is one atomic frame. |

---

## App Spawning

`spawn_app` is the composition primitive: one app asks Plexi to launch another app and place it in a layout slot relative to the caller's pane. This is how cross-app flows are built in v1 — e.g. a Rust file browser pressing Enter on a `.txt` file emits `spawn_app` and a Python text editor opens in a 50/50 right split, lifecycle-bonded so closing the file browser closes the editor too.

### Request shape

```json
{
  "type": "spawn_app",
  "app_id": "text-editor",
  "args": ["/tmp/notes.txt"],
  "parent": "self",
  "layout": { "kind": "cols", "slot": 1, "ratio": 0.5 },
  "lifecycle": "cascade",
  "linked": true,
  "wire_channels": ["file_buffer"]
}
```

### Fields

| Field | Type | Default | Meaning |
|---|---|---|---|
| `app_id` | string | **required** | The id of the app to spawn. Must exist in Plexi's app registry at spawn time; unknown ids are refused with a warning notification to the caller. |
| `args` | string array | `[]` | Command-line args forwarded to the spawned app as `argv[1..]`. Identical to how the palette / CLI pass file arguments. |
| `parent` | string | `"self"` | Anchor for the new pane's position. `"self"` = the emitting pane; `"root"` = top-level (ignores the emitter's location entirely); `"mark:<name>"` = reserved for a future named-layout system. |
| `layout` | object | `{ "kind": "fill" }` | How to position the new pane relative to `parent`. See the layout enum below. Falls back to the target's `[app.spawnable].default_layout` when omitted by the caller. |
| `lifecycle` | string | `"cascade"` | `cascade` closes the child when the parent closes, `orphan` detaches it, `prompt` asks the user (see the v1 stub below). |
| `linked` | bool | `true` | When true, the new pane joins the parent's linked-pane group so terminal-linking is shared. |
| `wire_channels` | string array | `[]` | Typed-pipe channel names to pre-wire between parent and child. Stored on the resulting spawn relationship for the typed-pipes spec to consume; the linking matrix is a separate spec and is not executed in v1. |

### Layout enum

`layout` is an object with a `kind` discriminator:

| `kind` | Additional fields | Meaning |
|---|---|---|
| `fill` | — | Fill the parent slot. If the parent is fullscreen, the new pane replaces it; otherwise it is placed into the same slot as the parent. |
| `cols` | `slot: 0 \| 1`, `ratio: 0.0..1.0 = 0.5` | Horizontal split. `slot = 0` = left, `slot = 1` = right. `ratio` is the fraction allocated to `slot`. |
| `rows` | `slot: 0 \| 1`, `ratio: 0.0..1.0 = 0.5` | Vertical split. `slot = 0` = top, `slot = 1` = bottom. |
| `grid_2x2` | `slot: 0..3` | 2×2 grid, row-major. **v1 stub:** the host accepts it but falls back to `fill` and logs a warning. |
| `custom` | `spec: any` | Forward-compat escape hatch. Any caller-supplied shape; v1 treats it as `fill` and logs. |

### Parent / child lifecycle

Every honored `spawn_app` call records a `SpawnRelationship { parent_pane, child_pane, lifecycle, wire_channels }` in an in-memory registry on `PlexiApp` (`SpawnRelationships` in `src/app_protocol.rs`). When a pane closes, Plexi consults this table and walks every relationship whose `parent_pane` matches:

- **`cascade`** — close each child too. The walk is recursive: a child that owns its own grandchildren closes them first. This is how the file-browser → editor flow collapses back to the original pane in one keystroke.
- **`orphan`** — drop the relationship and leave the child alive as a normal top-level pane.
- **`prompt`** — **v1 stub.** The host logs a warning and falls back to `orphan`. A future version will raise an interactive confirmation. Callers are free to request `prompt` today — they will just get orphan semantics until the prompt UI ships.

When a relationship is removed (child closed, or pane removed for any other reason), the relationship row is dropped via `SpawnRelationships::remove_pane`. Callers can look up children with `children_of(parent_id)` and walk up with `parent_of(child_id)`.

### Interaction with linked-pane groups

When `linked = true` (the default), the new pane joins the parent's linked-pane group so commands like `cd`, `run_in_terminal`, and future shared-terminal events flow between them. When `linked = false`, the child is a strict sibling in the tile tree with its own independent terminal affinity. The host reads the existing linked-pane mechanism (whatever the file browser already uses for its own split-on-open) and adds the child to it.

### Authorization (`[app.spawnable]`)

Before Plexi creates the child pane, it looks up the target `app_id` in the registry and consults the target's `[app.spawnable]` manifest table:

1. `allow_callers` — if the list does not contain `"*"` and does not contain the spawner's `app_id`, the spawn is refused. Plexi emits an error-level notification back to the caller via the standard notification channel (source = the caller's app id).
2. `allow_lifecycle` — must contain the requested lifecycle string (`"cascade"`, `"orphan"`, or `"prompt"`). If not, the spawn is refused with the same notification path.
3. If the caller omitted `layout`, the target's `default_layout` is applied.

Apps that do not declare `[app.spawnable]` at all inherit permissive defaults: any caller, any lifecycle, `fill` layout. Unknown `app_id`s are a warning, not a crash — the caller keeps running and sees a refusal notification.

### Canonical flow (file browser → text editor)

The canonical example — and the reason this primitive exists — is the file browser opening a file in a text editor:

1. User presses Enter on `notes.txt` in the Rust file browser.
2. File browser emits `spawn_app("text-editor", args=["notes.txt"], parent="self", layout={"kind":"cols","slot":1,"ratio":0.5}, lifecycle="cascade", linked=true)`.
3. Plexi looks up `text-editor` in the registry, checks `[app.spawnable]`, splits the file browser's pane horizontally at 50%, and launches the Python text editor in the right slot with `PLEXI_LAUNCH_MODE=spawned` and `PLEXI_PARENT_PANE=<browser pane id>` set.
4. The spawn relationship is recorded with `lifecycle = cascade`. When the user later closes the file browser, Plexi's pane-close path walks `children_of(browser_pane)` and closes the text editor too.

The file browser itself is not wired to emit `spawn_app` in v1 — that is a follow-up commit. This section documents the contract the protocol provides so the next change can plug in without renegotiating the shape.

---

## Capability system

Capabilities are declared at install time in `manifest.toml` under `[app.capabilities]`. At load time the host converts them into a runtime `AppPermissions` struct via `AppCapabilities::to_permissions` (`src/app_registry.rs`). All manifest-loaded apps are assigned the `sandboxed` trust level (built-in Rust apps get `builtin` and bypass checks).

### v1 enforcement

As of v1, the protocol itself has no structured filesystem, keychain, or network API. Apps run as ordinary subprocesses and can read/write anything the user's account can; the capability declarations are **advisory** for these dimensions. The fields that are actively enforced are:

- **`mouse_tracking`** — the host gates `mouse_move` event delivery on this flag (or the runtime `mouse_tracking` draw command). Apps that opt out will simply not receive the events.
- **`terminal_write`** — enforced at the app API layer: the host will refuse to translate `run_in_terminal` / `cd` draw commands from an app that didn't declare `terminal_write = true`.

`filesystem`, `env_file_access`, `network`, and `secrets_write` are recorded in the runtime permission struct and surfaced to the user in the Permissions Manager, but they are not enforced by any sandbox in v1. **Treat them as a manifest of intent, not a security boundary.**

Future versions will add a structured capability-gated API (`list_dir`, `read_file`, `write_file`, `secret_get`, `run_command`) on top of this manifest, using the same capability names. See "Non-goals" below for what is explicitly NOT in v1.

### Trust levels

| Level | Assigned to | Behavior |
|---|---|---|
| `builtin` | Rust apps compiled into the binary. | Bypass capability checks. |
| `trusted` | Reserved for user-elevated third-party apps. | Identical grants to `sandboxed` in v1; distinction is for future use. |
| `sandboxed` | All manifest-loaded apps. | Subject to the advisory capability checks described above. |

---

## State persistence

Plexi uses `get_state` / `set_state` + the `state` draw command for undo, redo, hot-reload continuity, and snapshot/restore. The protocol defines four state buckets that Plexi passes through untouched:

| Bucket | Meaning (convention) |
|---|---|
| `user_state` | The primary mutable state the user cares about undoing — selection, cursor, text content, etc. Always included in undo snapshots. |
| `derived` | Computed state that can be re-derived from `user_state`. Apps can store it for speed or omit it (recomputed on hydrate). |
| `session` | Transient, app-local state that matters for the current run but should not be undoable — scroll offsets, hover highlights, modal dialog state, etc. |
| `persistent` | State that should survive across app restarts — bookmarks, preferences, recent files. Apps are responsible for actually persisting this (Plexi currently does not flush it to disk on its own). |

### Undo / redo

When the user triggers undo in Plexi, the host sends `get_state` to the app and pushes the response onto the redo stack, then pops the undo stack and sends a `set_state` with the popped state. The redo case is symmetric. The app sees only `set_state` events and does not need to implement undo internally.

The undo stack is capped at `MAX_UNDO_DEPTH = 50` entries; the oldest entry is dropped when the cap is exceeded. Redo is cleared whenever a new user action pushes onto the undo stack.

### Hydration contract

The app MUST respond to `get_state` with exactly one `state` command. If the handler is not implemented, respond with `{ "type": "state", "user_state": {}, "derived": {}, "session": {}, "persistent": {} }` — the Python SDK's default `_handle_get_state` does this automatically when no `@app.on_get_state` handler is registered.

---

## Cost reporting

Apps that call LLM APIs report their costs back to Plexi via the `cost_report` draw command. The host:

1. Accumulates `cost_usd` into an in-memory session total attached to the `ProcessApp` (`session_cost_usd()`).
2. Logs a human-readable info line to the Plexi log: `app::<id> cost: $X.XXXX (<service> <model> in:N out:M)`.
3. Appends a JSON line to `<config_dir>/costs.jsonl` containing `app_id, service, model, input_tokens, output_tokens, cost_usd, operation_id, timestamp`. If the timestamp field is absent, the host substitutes `Utc::now()` (RFC 3339). The parent directory is created on first write.

The Python SDK exposes this as `Emitter.cost_report(service, model, input_tokens, output_tokens, cost_usd, operation_id=None)`, which automatically fills `app_id` from the emitter and stamps `timestamp = datetime.now(timezone.utc).isoformat()` and `operation_id = uuid4()` if not supplied.

The session total is reset when the process exits. Daily / cumulative totals must be derived from `costs.jsonl` by downstream tooling.

---

## Feedback primitive

Apps can write user-submitted feedback to a per-app JSONL file via the Python SDK `Emitter.submit_feedback(text, rating=None, category=None)` helper. **This is a client-side convenience, not a draw command** — it bypasses the protocol entirely and writes directly from the app process.

- File location: `$PLEXI_APPS_DIR/$PLEXI_APP_ID/feedback.jsonl`, falling back to `~/.plexi/apps/<app_id>/feedback.jsonl` if the env vars are missing.
- Entry schema: `{"ts": <RFC 3339>, "text": <string>, "rating": <int?>, "category": <string?>}`
- The SDK also emits an `info` log on success and a `warn` log on `OSError`. The feedback file's parent directory is created on first write.

Introduced in Python SDK 0.2.0. The Rust SDK does not currently implement a feedback helper; Rust apps can write the same file manually. Because the app writes the file directly, it inherits filesystem permissions from the process — no capability check is applied.

---

## Logging protocol

### Structured `log` command

Apps emit `{"type":"log","level":"…","message":"…"}` and Plexi logs the line with target `app::<app_id>` at the requested level. Unknown levels default to `info`. The log flows to the usual destinations:

| Build | Log file |
|---|---|
| Alpha | `~/.plexi-alpha/plexi.log` |
| Beta | `~/.plexi-beta/plexi.log` |
| Stable | `~/.plexi/plexi.log` |

Log level is controlled by `[log].level` in `config.toml`. Third-party crates (egui, wgpu, etc.) are always clamped to `warn` regardless.

### Stderr forwarding

Every byte an app writes to stderr is captured by Plexi on a background thread and forwarded into the Plexi log as a `warn`-level entry with target `app::<app_id>`. Each non-empty line becomes one log entry prefixed with `stderr:`. This means Python tracebacks, Rust panics, and anything else a subprocess logs on stderr will appear in `plexi.log` automatically — apps don't need to emit `log` draw commands for them.

### Python SDK log helpers

`RenderContext` and `Emitter` both expose `info(msg)`, `warn(msg)`, `error(msg)`, `debug(msg)`, and `log(level, msg)`. Inside a render frame, calls on `ctx` accumulate in the frame and are flushed with the other commands; calls on `emit` are written and flushed immediately.

---

## App lifecycle & hot reload

### Spawn

On launch, Plexi:

1. Reads `manifest.toml` from the app install dir.
2. Resolves the `entry` path and verifies it is executable.
3. For `.py` entries, picks a concrete Python interpreter (see "Install layout").
4. Spawns the process with `stdin`/`stdout`/`stderr` piped, `current_dir = <launch cwd>`, and `PLEXI_APP_ID` / `PLEXI_APPS_DIR` set.
5. Starts two background reader threads — one for stdout (draw commands) and one for stderr (log forwarding).
6. Starts a file-change watcher on the app's parent directory (hot reload).
7. On the first `ui()` tick, sends `init`, records `last_size`, and begins the render loop.

### Hot reload

Every launched app has a recursive file watcher on its install directory. Any change to a `.py` file (create, modify, rename) triggers a debounced reload:

1. The reload channel is drained; if at least one signal arrived AND more than 200 ms has passed since the last reload, a reload is executed.
2. Plexi sends `shutdown` to the current process, waits briefly, then SIGKILLs it.
3. The child is respawned with the same entry path, cwd, and args; new stdout/stderr reader threads start; `frame`/`pending_frame` buffers are cleared.
4. The new process is re-initialized (`init` sent on the next `ui()` tick). Mouse tracking state resets to `false` — the new process must opt in again.
5. **State is not automatically handed off.** If an app wants to survive reloads, it must persist its state in its own `persistent` bucket and re-hydrate on `init` / first `render`. Plexi does not proactively snapshot before a reload in v1.

The watcher filters on the `.py` extension — reloads only fire for Python source changes. Non-Python apps will not hot reload in v1.

See issue #83 for the hot-reload feature discussion.

### Shutdown

When a pane closes (or an app is replaced), Plexi:

1. Sends a `shutdown` event to the app.
2. Drops the `ProcessApp`, which attempts a non-blocking `wait()` and then SIGKILLs as a belt-and-braces guarantee.
3. The child's stderr reader thread sees EOF and exits.

Apps SHOULD break their event loop as soon as they receive `shutdown`. Apps MUST NOT rely on receiving it — if Plexi crashes, the subprocess will orphan and receive SIGHUP/SIGPIPE.

---

## Self-closing panes (OSC 0 convention)

Any process running in a Plexi terminal pane — not just a Plexi app — can close its containing pane by emitting an OSC 0 "set window title" escape sequence with the literal title `plexi:close`:

```sh
printf '\e]0;plexi:close\x07'
```

The terminal backend parses OSC 0 as a title-change event, Plexi intercepts the `PtyEvent::Title` in the main loop, strips the `plexi:` prefix, and — for the recognized command `close` — queues the containing pane for removal. Unknown `plexi:*` commands are logged at debug level and ignored.

This is a convention, not a protocol: it is driven entirely from shell output and does not require a running app. A CLI script can close its own pane without any SDK. See issue #90.

---

## Error handling

### Malformed events (Plexi → App)

If the host sends an event the app can't parse, the app SHOULD silently ignore the line and continue reading. The Python SDK does exactly this: a `JSONDecodeError` on an input line causes the line to be skipped. This lets Plexi add new event types without breaking older apps.

### Malformed draw commands (App → Plexi)

If the app sends a line that doesn't parse as a valid `DrawCommand`, Plexi logs a `warn` with the parse error and the offending line, then continues reading. The offending line is dropped; the next line is parsed normally. A single bad command does not take down the frame or the app.

### Partial frames

If the process exits mid-frame (before emitting `frame_done`), the last committed frame is still displayed — the pending accumulator is discarded when the host drops the `ProcessApp`. Users see the last good state until the pane is explicitly closed.

### Protocol-level panics

A Rust panic or Python traceback on the app side writes to stderr and terminates the process. The stderr lines flow into `plexi.log` as `warn` entries tagged `app::<id>`. Plexi's reader thread sees EOF on stdout, the frame buffer freezes, and the pane displays the last good frame. Plexi does not automatically restart panicked apps — the user (or hot-reload) has to trigger a respawn.

### Slow apps

No timeout. If an app takes longer than one frame to respond, Plexi just repaints the last good frame and requeues `render` on the next tick. Slow is not fatal.

---

## Versioning & stability

This spec is **v1 — stable**. Every event, draw command, field name, default value, env var, file path, manifest key, and behavior documented here is committed for the v1 contract. Changes in future versions will be staged:

- **Additive** changes (new events, new draw commands, new optional fields) may land in any minor release. Apps MUST ignore unknown events and MUST ignore unknown fields on known events to stay forward-compatible.
- **Renames and deprecations** will be staged: the new name ships first, both names work, a release later a `warn` is logged on use of the old name, and only then is the old name removed. At minimum one full minor version of "both work" before `warn`, and one more before removal.
- **Breaking changes** will not happen inside v1. A breaking change requires a v2 protocol.

Apps can detect the protocol version informally by reading Plexi's own version (from CLI or the about dialog). The protocol does not ship a version number on the wire in v1 — this is a deliberate simplification for the single-producer single-consumer model.

---

## Non-goals

The following are explicitly **not** part of v1 and should not be implemented against. Reasoning is included so future work can re-open the question cleanly.

- **Structured filesystem API.** No `list_dir` / `read_file` / `write_file` events. Apps run as full subprocesses and have raw filesystem access via their language's stdlib. A structured API is desirable for sandboxing but adds a large surface area that would slow v1.
- **Structured secrets API.** No `secret_get` / `secret_store` events. Apps that need secrets read them from env vars or their own config files. Plexi's secrets manager is a separate subsystem that will grow its own API.
- **Sandboxing / process confinement.** No `sandbox_init` on macOS, no `seccomp-bpf` on Linux. Capability flags in `manifest.toml` are advisory for filesystem/network. Real sandboxing requires OS hooks that are explicitly deferred.
- **Graphical compositor access.** No OpenGL / Metal / wgpu handles exposed to apps. Apps must render through the `DrawCommand` primitives — this is what makes the protocol portable to future backends (WASM, remote rendering).
- **Direct GPU or texture creation.** Apps cannot upload their own textures; they reference files on disk and let Plexi manage the image / video caches.
- **Multi-window apps.** One app = one pane. Apps cannot open additional windows or child panes in v1. A companion terminal pane is the only secondary surface and is declared statically in the manifest.
- **WASM runtime.** Apps must be native executables in v1. Browser/WASM deployment is the v3+ endgame.
- **Typed click event.** The legacy `click` event is defined in the protocol but not emitted by the v1 host. Apps should handle mouse input through `mouse_down` / `mouse_up` and ignore `click`. The type remains reserved.
- **Two-way request/response RPC.** Every message is fire-and-forget (apart from `get_state`, which has a bounded one-shot response contract). Apps cannot query Plexi synchronously for configuration, theme colors, or terminal state in v1.

---

## See also

- **Shipping code (source of truth):**
  - `src/app_protocol.rs` — `PlexiEvent`, `DrawCommand`, `Modifiers`, `MouseButton`, `ListItem`.
  - `src/process_app.rs` — spawn, init, frame loop, stdout/stderr readers, hot reload, key forwarding, undo/redo state machine.
  - `src/app_registry.rs` — `manifest.toml` parser, install directory scan, `[app.launch]` companion config.
  - `src/app_permissions.rs` — `AppPermissions`, `TrustLevel`, `FsPermission`.
  - `src/cost_tracker.rs` — cost accumulation and `costs.jsonl` append.
  - `src/app.rs` — OSC 0 `plexi:close` handling (`PtyEvent::Title`).
  - `src/tiling.rs` — mouse button / scroll forwarding from egui events to `PlexiEvent`.
- **SDKs:**
  - `sdk/python/plexi_sdk.py` — canonical Python SDK. Zero deps. Version 0.2.0.
  - `sdk/python/plexi_sdk_advanced.py` — Canvas transform, HitTester, FrameTimer, Tween helpers. Does not add protocol surface — pure client-side convenience on top of the base SDK.
  - `sdk/rust/src/lib.rs` — Rust SDK. Implements the App trait and a blocking `run()` event loop. Covers the stable subset of the protocol; lags the Python SDK on newer additions (no `scroll`, `mouse_down`/`up`, `drop`, `get_state`, `cost_report`, `notification`, `feedback`, `log` yet).
- **Sibling specs (non-duplicative):**
  - `docs/specs/proposals/core-advanced-ui-sdk.md` — Canvas transform, hit testing, tween helpers, frame timing. Everything client-side.
  - `docs/specs/proposals/core-text-editor-primitive.md` — proposed text-editor primitive (not part of v1 protocol).
  - `docs/specs/proposals/chat-primitive.md` — proposed chat primitive (not part of v1 protocol).
  - `docs/specs/proposals/core-layout-presets.md` — the companion-pane model underlying `[app.launch]`.
  - `docs/specs/subsystems/intelligence-protocol.md` — deferred intelligence/LLM routing layer. Apps currently call providers directly and use `cost_report`.
  - `docs/specs/proposals/agent-replay-testing.md` — consumes `costs.jsonl` for run attribution.
- **GitHub issues referenced:** #83 (hot reload), #90 (OSC 0 self-close).

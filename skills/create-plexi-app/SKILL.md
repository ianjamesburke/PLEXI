---
name: create-plexi-app
description: Use when building, scaffolding, or modifying a Plexi Python app. Covers manifest, SDK surface, key-handling, the dev loop, render verification, and logging requirements.
skill_version: "0.0.461"
---

# Build a Plexi App

**Keeping this skill current:** Any PR that changes the SDK surface (`sdk/python/plexi_sdk/`), adds a new draw command, or modifies the manifest schema must also bump `skill_version` in this file's frontmatter and update the affected sections. The ship-issue cycle enforces this — see the "SDK changes" rule in `.claude/skills/ship-issue/SKILL.md`.

---

## Channel / binary

Throughout this skill, `<plexi-binary>` means the binary for your active development channel:

| Channel | Binary | Log |
|---|---|---|
| Alpha | `plexi-alpha` | `~/.plexi-alpha/plexi.log` |
| Beta | `plexi-beta` | `~/.plexi-beta/plexi.log` |
| Stable | `plexi` | `~/.plexi/plexi.log` |
| PR build | `plexi-pr-<N>` | `~/.plexi-pr-<N>/plexi.log` |

Substitute the correct binary and log path everywhere in this skill. Default to `plexi-alpha` for active development.

---

## Scaffold (always use `<plexi-binary> app init`)

Never hand-write a `manifest.toml`. Always scaffold:

```bash
<plexi-binary> app init <app-name>
```

This creates `<cwd>/.plexi/apps/<app-name>/` with a valid manifest and `main.py`. The app is immediately registered in the workspace — no additional install step needed. Editing an init-generated manifest is fine; writing one from scratch produces subtle schema errors.

After scaffolding, prune unused imports from `main.py` before implementing. The template imports several UI components; remove any you won't use (Pyright will flag them).

---

## Manifest (`manifest.toml`)

```toml
schema_version = 1

[app]
id = "my_app"           # snake_case, unique
type = "app"
name = "My App"
version = "0.1.0"
description = "One line."
entry = "main.py"

[app.capabilities]
capabilities = []       # add: "net", "audio", "midi", "video" as needed

[launch]
layout_hint = { side = "right", split = 0.45 }
```

`layout_hint.side`: `"right"` | `"left"` | `"bottom"` | `"top"`. `split` is a 0.0–1.0 fraction of the parent.

---

## App skeleton

```python
from plexi_sdk import App, BG, FG, BODY, ACCENT, SURFACE, MUTED
from plexi_sdk import HEADING, CAPTION, PAD, PAD_TIGHT

class MyApp(App):
    async def on_init(self, ctx):
        self.emit.info("MyApp ready")   # required — first info trace

    def on_render(self, ctx):
        ctx.clear(BG)
        # draw here

    def on_key(self, ctx, key, mods):
        # key: "a"-"z", "up"/"down"/"left"/"right", "return", "escape",
        #      "backspace", "tab", "space", "f1"-"f12"
        # mods: {"shift": bool, "ctrl": bool, "alt": bool, "meta": bool}
        pass

MyApp().run()
```

---

## SDK surface map

### Drawing (`ctx` in `on_render`)

| Method | Purpose |
|---|---|
| `ctx.clear(fill)` | Full-pane background fill |
| `ctx.rect(x,y,w,h, fill, radius=0)` | Filled rectangle |
| `ctx.text(x,y, text, size, color, monospace, bold, align, max_width, selectable)` | Text; `align`: `"top_left"` / `"center"` / `"top_center"` / `"right"` |
| `ctx.markdown(x,y,w, text)` | Host-rendered markdown |
| `ctx.image(src, x,y,w,h, fit)` | Image from path or URL |
| `ctx.circle(cx,cy,r, fill)` | Filled circle |
| `ctx.arc(cx,cy,r, start, end, fill)` | Pie slice, radians |
| `ctx.line(x1,y1, x2,y2, color, width)` | Line |
| `ctx.badge(x,y_center, label)` | Pill badge |
| `ctx.button(id, x,y,w,h, label)` | Clickable button (use `ctx._clicks` to detect) |
| `ctx.list_view(items, selected)` | Scrollable list |
| `ctx.begin_scroll / ctx.end_scroll` | Scroll region |
| `ctx.text_input(id, x,y,w, value, placeholder)` | Single-line text input |
| `ctx.push_clip / ctx.pop_clip` | Clip region |
| `ctx.shortcuts(x,y,max_width, shortcuts)` | Key shortcut hint row |
| `await ctx.measure_text(text, size, monospace)` | Measure text width (async; use sparingly) |
| `ctx.info/warn/error/debug(msg)` | Log from render |
| `ctx.notify(title, body)` | Fire-and-forget notification |
| `ctx.schedule_render(after_ms)` | Request re-render after delay |

**Prefer `ctx.render(tree)` from `plexi_sdk.ui`** for standard layouts — use raw draw calls only for custom visuals.

### Higher-level UI (`from plexi_sdk.ui import ...`)

`Column`, `Row`, `Header`, `Card`, `KeyRow`, `Spacer`, `Footer`, `Divider`, `Label`, `Badge`, `Tabs` — see `docs/sdk-ui-guide.md` and `examples/ui-playground/`.

```python
from plexi_sdk.ui import Column, Header, Card, KeyRow, Footer
ctx.render(Column([
    Header("Title", "subtitle"),
    Card([KeyRow("q", "Quit")]),
    Footer("hint"),
]))
```

### Widgets (`from plexi_sdk.widgets import ...`)

`ScrollState`, `TextBuffer`, `Cursor`, `Selection`, `TextArea`, `TextAreaTheme`, `emit_text_input`, `Button`, `ButtonStyle`, `KeyMap`

### Emitter (`self.emit` / `ctx.emit`, available in all handlers)

| Method | Purpose |
|---|---|
| `emit.info/warn/error/debug(msg)` | Log to host log |
| `emit.notify(title, body, level)` | Notification |
| `await emit.notify_choice(title, options)` | Notification with choices |
| `await emit.notify_input(title, prompt)` | Text-input notification |
| `await emit.secret_get(key)` | Fetch secret by key |
| `await emit.http_request(url, method, headers, body)` | HTTP (requires `net` capability) |
| `await emit.ai_query(model_tier, system, messages)` | AI query via host broker |
| `emit.spawn_pane(type_id, layout)` | Open another pane |
| `emit.push_nav(view_id, title)` / `emit.pop_nav()` | Navigation stack |
| `emit.cd(path)` / `emit.run_in_terminal(cmd)` | Shell integration |
| `emit.status_summary(text)` | Update status bar |
| `emit.set_timer(id, after_ms)` / `emit.cancel_timer(id)` | Timers → fires `on_timer(ctx, id)` |
| `emit.set_mouse_tracking(enabled)` | Raw mouse move events |
| `emit.audio_play(src)` | Play audio file |
| `await emit.list_audio_devices()` / `await emit.list_midi_devices()` | Device enumeration |
| `emit.load_state()` / `emit.save_state(dict)` | Persist app state across sessions |
| `emit.copy_to_clipboard(text)` | Write to clipboard |

### Constants (`from plexi_sdk import ...`)

**Colors (Catppuccin Mocha):** `BG`, `SURFACE`, `HIGHLIGHT`, `ACCENT`, `MUTED`, `FG`, `RED`, `GREEN`, `YELLOW`  
**Font sizes:** `TITLE=22`, `HEADING=18`, `BODY=15`, `CAPTION=13`, `HINT=12`, `MONO_BODY=14`, `MONO_SMALL=12`  
**Layout:** `PAD=16`, `PAD_TIGHT=8`, `HEADER_H=48`, `STATUS_H=44`  
**Notification priorities:** `PRIORITY_LOW=0`, `PRIORITY_NORMAL=50`, `PRIORITY_HIGH=100`, `PRIORITY_CRITICAL=200`  
**Helpers:** `rgba(r,g,b,a)`, `dim(hex, alpha)`

---

## Key-handling philosophy

- `escape` → go back / cancel / dismiss (never exit — host owns quit)
- `return` → confirm / submit
- `up` / `down` → navigate lists
- `left` / `right` → prev/next tab or item
- `q` → contextual quit-like action only if clearly documented in the UI
- Arrow keys sent as `"up"`, `"down"`, `"left"`, `"right"` (SDK normalizes from egui's `ArrowLeft` etc.)
- `mods["meta"]` = ⌘ on macOS; prefer `meta` over `ctrl` for primary actions

---

## Lifecycle hooks

```
on_init(ctx)              async, awaited — one-time setup after handshake
on_render(ctx)            sync or async, awaited — draw every frame
on_key(ctx, key, mods)    task — dispatched concurrently
on_click(ctx, x, y, btn)  task
on_command(ctx, text)     task — user typed a command
on_paste(ctx, text)       task
on_pipe_message(ctx, id, payload)  task
on_path_changed(ctx, cwd) task
on_suspend / on_resume    async, awaited
on_shutdown               async, awaited
on_scroll(ctx, id, offset_y)  task — from begin_scroll regions
on_timer(ctx, timer_id)   task — from emit.set_timer()
```

**task** handlers: dispatched as asyncio tasks — event loop doesn't wait. Declare `async def` for any I/O. Never call `time.sleep` or blocking requests in them; use `await asyncio.to_thread(fn)`.

---

## Dev loop

```bash
# 1. Scaffold (app lands in <cwd>/.plexi/apps/<app-name>/ — registered immediately):
<plexi-binary> app init <app-name>

# 2. Implement main.py

# 3. Render to PNG and visually verify — REQUIRED before surfacing to user:
<plexi-binary> app render <app-id> --output /tmp/<app-id>.png
# Read /tmp/<app-id>.png with the Read tool. Inspect layout, text, colors.
# Fix issues, re-render, repeat until the screenshot looks correct.

# 4. Only after render passes — open for the user:
<plexi-binary> open <app-id> --layout split_h

# Tail logs:
tail -f ~/.plexi-<channel>/plexi.log
```

**Render verify is mandatory.** Never surface an app to the user without first rendering it headlessly, reading the PNG, and confirming it looks correct. This is the agent's quality gate — not an optional step.

**`app render` reads from the workspace where the app is registered** (the `<cwd>/.plexi/apps/` dir). Run it from the same directory you ran `app init`. If the render fails with a "skipping" warning, the app isn't registered — confirm you're running from the right directory.

**Pyright variance warnings on SDK list types are benign.** `List[Component]` is invariant; passing `list[Label]` to `Card([...])` triggers Pyright but works at runtime. Do not restructure correct code to silence these warnings.

**Never run `<plexi-binary> open` from a Claude Code pane** — it blocks the session. Instruct the user to open it from a separate terminal pane. The `layout_hint` in the manifest handles positioning automatically.

---

## Logging requirements (not optional)

Every app must emit at least one `info`-level trace per meaningful state change:

- `on_init` → `self.emit.info("<AppName> ready")` — required
- Key actions that change state → `self.emit.info("action: <what>")`
- Errors → `self.emit.error("context: <what failed>")`
- Use `ctx.info()` / `emit.info()` — not `print()` (stdout is the host protocol pipe)

---

## Testing

### SDK snapshot tests (`from plexi_sdk.testing import ...`)

Unit-level: render raw draw commands to PNG and assert pixel values:

```python
from plexi_sdk.testing import render_draw_commands, assert_pixel, assert_color_present

png = render_draw_commands([
    {"type": "rect", "x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "radius": 0},
    {"type": "text", "x": 10, "y": 10, "text": "hi", "size": 14.0, "color": "#ffffff",
     "monospace": False, "bold": False, "align": "top_left", "max_width": None,
     "elide": True, "selectable": False},
], width=200, height=200)

assert_pixel(png, x=50, y=50, expected=(255, 0, 0, 255))
assert_color_present(png, (255, 0, 0, 255))
```

Run with: `uv run pytest sdk/python/tests/`

Full `AppHarness` (headless event injection) is coming — see issue #865.

---

## Layout Safety Rules

### Rule 1 — Right-edge text (hint/breadcrumb rows)

Never place hint or breadcrumb text at a fixed coordinate near the right edge. It clips silently when the pane is narrower than the assumed minimum width — the left clip is the non-obvious failure.

```python
# w = ctx.width (pane width)
# Bad:  ctx.text(w - 260, y, hint, CAPTION, MUTED)                          # clips when pane is narrow
# Good: ctx.text(max(w / 2, w - 260), y, hint, CAPTION, MUTED, align="right")
# Or:   ctx.text(w - PAD, y, hint, CAPTION, MUTED, align="right")           # right-align from edge
```

### Rule 2 — Minimum text alpha

Any alpha below 160 is effectively invisible on Catppuccin Mocha dark backgrounds at typical pane sizes.

```python
# dim(FG, 80)  → invisible on dark bg — never use for text
# dim(FG, 120) → barely readable — avoid
# dim(FG, 160) → minimum for de-emphasized but readable text
# dim(FG, 200) → standard secondary text
# FG (255)     → primary / focused text
```

---

## Common mistakes

| Mistake | Fix |
|---|---|
| Surfacing app to user without rendering first | Always `<plexi-binary> app render <app-id> --output /tmp/<id>.png`, Read the PNG, verify visually before opening |
| `print()` inside handlers | Use `self.emit.info()` or `ctx.info()` — stdout is the host protocol pipe |
| `time.sleep()` in a task handler | `await asyncio.sleep()` or `await asyncio.to_thread(fn)` |
| Hand-writing `manifest.toml` | Always `<plexi-binary> app init <name>` |
| Hard-coding pixel sizes for text | Use `await ctx.measure_text()` or `ctx.render()` with UI components |
| No logging | Every app ships with at minimum `on_init` log and error traces |
| Using `ctrl` for primary shortcuts | Prefer `meta` (⌘) on macOS |
| `ctx.text(w - N, y, hint, CAPTION, MUTED)` near right edge | Use `ctx.text(max(w/2, w - N), y, hint, CAPTION, MUTED, align="right")` — prevents left-clip in narrow panes |
| `dim(FG, 80)` or `dim(FG, 120)` for labels | Minimum readable alpha is 160 — use `dim(FG, 160)` for de-emphasized text |
| Pyright variance warnings on `list[Label]` in `Card([...])` | Benign — `List[Component]` invariance is a type stub issue, not a runtime error. Do not restructure correct code to silence it. |

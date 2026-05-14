---
name: create-plexi-app
description: Use when building, scaffolding, or modifying a Plexi Python app. Covers manifest, SDK surface, key-handling, the dev loop, testing, and logging requirements.
skill_version: "3.5.122"
---

# Build a Plexi App

**Plexi version this skill was written against:** `3.5.122`

Check drift before using: `<plexi-binary> --version`. If the version differs, this skill may be stale — re-read the SDK source and update it.

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
chmod +x <app-name>/app.py
```

This produces a valid manifest with the correct `schema_version`. Editing an init-generated manifest is fine; writing one from scratch produces subtle schema errors.

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
entry = "app.py"

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
# Start hot-reload dev server (rebuilds on file save):
just dev <app-dir>

# Launch in a new split pane (--layout prevents reusing/replacing a running instance):
<plexi-binary> open <app-id> --layout split_v

# Headless render to PNG — no running host required:
# Agent runs this, reads the PNG with the Read tool, iterates on code, re-renders to confirm.
<plexi-binary> app render <app-id> --output /tmp/render.png

**Before `app render` works**, the app must be registered. If no workspace exists yet:
```bash
<plexi-binary> workspace init   # run once from the repo root
<plexi-binary> app link <app-dir>   # registers without moving files
```
`app link` is idempotent — safe to re-run.

# Tail logs:
tail -f ~/.plexi-<channel>/plexi.log
```

**Never run `<plexi-binary> open` or `just dev` from a Claude Code pane** — it blocks the session. Use a separate terminal pane or instruct the user to open it. The `layout_hint` in the manifest handles the side-split automatically.

---

## Logging requirements (not optional)

Every app must emit at least one `info`-level trace per meaningful state change:

- `on_init` → `self.emit.info("<AppName> ready")` — required
- Key actions that change state → `self.emit.info("action: <what>")`
- Errors → `self.emit.error("context: <what failed>")`
- Use `ctx.info()` / `emit.info()` — not `print()` (stdout is the host protocol pipe)

---

## Testing

### CLI render (agent feedback loop)

Render the app to PNG headlessly — no running host required. The agent runs this, reads the PNG with the Read tool, edits the code, and re-renders to confirm. The user never needs to open the image.

```bash
<plexi-binary> app render <app-id> --output /tmp/render.png
# then: Read /tmp/render.png to inspect visually, edit code, re-render
```

**Critical limitation: `app render` always pulls from the registered workspace (repo root), not from a worktree.** Re-rendering from a feature worktree will not pick up uncommitted changes — the binary finds the app via the workspace registry, not CWD. Options:
- **For layout/design iteration on alpha directly:** edit in place, render, verify, then move to a worktree for the PR.
- **For worktree changes:** install a PR build first (`just pr-install <N>`), then use `plexi-pr-<N> app render <app-id>`. Don't try to render worktree changes without a PR install — it will silently show stale output.

If the render fails with a "skipping" warning, the app isn't registered — verify the manifest path and that the workspace or examples dir is indexed.

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

## Common mistakes

| Mistake | Fix |
|---|---|
| `print()` inside handlers | Use `self.emit.info()` or `ctx.info()` — stdout is the host protocol pipe |
| `time.sleep()` in a task handler | `await asyncio.sleep()` or `await asyncio.to_thread(fn)` |
| Hand-writing `manifest.toml` | Always `<plexi-binary> app init <name>` |
| Hard-coding pixel sizes for text | Use `await ctx.measure_text()` or `ctx.render()` with UI components |
| No logging | Every app ships with at minimum `on_init` log and error traces |
| Using `ctrl` for primary shortcuts | Prefer `meta` (⌘) on macOS |

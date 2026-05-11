#!/usr/bin/env python3
"""descriptor-renderer — auto-UI renderer for --plexi CLI descriptors (issue #361).

Accepts a descriptor path as the first argv or --descriptor <path>.
Requests a linked terminal at startup, then renders the CLI's command
tree as a clickable GUI that drives the terminal.

List view:   j/k or ↑/↓ move selection; Enter opens; Escape goes back.
             Selected row is highlighted. Scroll auto-tracks the cursor.
Form view:   host-managed text_input per arg/flag; first field auto-focused;
             Tab cycles fields; Enter on last field runs the command.
             Escape returns to list.
"""

import json
import sys
import threading
from pathlib import Path

from plexi_sdk import App, RenderContext, CapabilityDeniedError
from plexi_sdk.ui import (
    Column, AppBar, FormField, FooterKeys,
    ListItem, Label, Spacer, Card,
    BG, HIGHLIGHT, ACCENT, MUTED, RED,
    TEXT_HINT,
    SPACE_XS, SPACE_SM, SPACE_MD, SPACE_LG,
    RADIUS_SM,
)
from plexi_sdk.widgets import ListView


# ── Helpers ────────────────────────────────────────────────────────────────────

def _parse_descriptor_path(argv: list[str]) -> str | None:
    i = 0
    while i < len(argv):
        if argv[i] == "--descriptor" and i + 1 < len(argv):
            return argv[i + 1]
        i += 1
    for arg in argv:
        if not arg.startswith("-"):
            return arg
    sample = Path(__file__).parent / "sample.json"
    if sample.exists():
        return str(sample)
    return None


def _load_descriptor(path: str) -> dict:
    p = Path(path).expanduser().resolve()
    return json.loads(p.read_text())


def _commands_at(descriptor: dict, path: list[str]) -> list[dict]:
    cmds = descriptor.get("commands", [])
    for segment in path:
        match = next((c for c in cmds if c["name"] == segment), None)
        if match is None:
            return []
        cmds = match.get("commands", [])
    return cmds


def _shell_quote(s: str) -> str:
    if not s:
        return "''"
    safe = all(c.isalnum() or c in "/._-:@+" for c in s)
    return s if safe else "'" + s.replace("'", "'\\''") + "'"


def _build_command(cli: str, path: list[str], field_values: dict[str, str],
                   fields: list[dict]) -> str:
    parts = [cli] + list(path)
    for f in fields:
        name = f["name"]
        val = field_values.get(name, "").strip()
        ftype = f.get("type", "string")
        if not val:
            continue
        if name.startswith("--") and ftype == "bool":
            if val.lower() in ("true", "1", "yes"):
                parts.append(name)
        elif name.startswith("--"):
            parts.append(f"{name}={_shell_quote(val)}")
        else:
            parts.append(_shell_quote(val))
    return " ".join(parts)


# ── App ────────────────────────────────────────────────────────────────────────

class DescriptorRendererApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self._descriptor: dict | None = None
        self._error: str = ""
        self._terminal_pane_id: int = 0
        # Navigation state
        self._view: str = "loading"   # loading | error | list | form
        self._cmd_path: list[str] = []
        self._selected_idx: int = 0   # cursor in current command list
        # Form state
        self._fields: list[dict] = []
        self._field_values: dict[str, str] = {}
        self._form_fields: list[FormField] = []
        self._last_run: str = ""
        # Hit regions: (y_top, y_bot, tag)
        self._hits: list[tuple[float, float, object]] = []
        # UI components
        self._list = ListView(item_height=ListItem.HEIGHT_DOUBLE)

        path = _parse_descriptor_path(sys.argv[1:])
        if not path:
            self._error = "No descriptor path. Usage: descriptor-renderer --descriptor <path.json>"
            self._view = "error"
            ctx.status_summary("descriptor-renderer: no path")
            return

        try:
            self._descriptor = _load_descriptor(path)
        except FileNotFoundError:
            self._error = f"File not found: {path}"
            self._view = "error"
            ctx.status_summary("descriptor-renderer: file not found")
            return
        except json.JSONDecodeError as e:
            self._error = f"Invalid JSON in {path}: {e}"
            self._view = "error"
            ctx.status_summary("descriptor-renderer: bad JSON")
            return
        except Exception as e:
            self._error = str(e)
            self._view = "error"
            ctx.status_summary(f"descriptor-renderer: {e}")
            return

        cli = self._descriptor.get("name", "?")
        ver = self._descriptor.get("version", "")
        ctx.status_summary(f"{cli} {ver}  ·  connecting terminal…")
        self._view = "list"
        self.emit.info(f"descriptor-renderer: loaded {path!r}  cli={cli!r}  ver={ver!r}")

        # request_linked_terminal blocks on the event-loop queue — must run
        # on a background thread so the loop stays free to read the response.
        threading.Thread(target=self._connect_terminal,
                         args=(cli, ver), daemon=True).start()

    def _connect_terminal(self, cli: str, ver: str) -> None:
        try:
            pane_id = self.emit.run_sync(
                self.emit.request_linked_terminal(cwd=None, label=f"{cli} terminal")
            )
            self._terminal_pane_id = pane_id
            self.emit.info(f"descriptor-renderer: linked terminal #{pane_id}")
        except CapabilityDeniedError as e:
            self.emit.warn(f"descriptor-renderer: terminal.bindings denied: {e}")
        self.emit.schedule_render(after_ms=16)

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        if self._view == "loading":
            ctx.render(Column([
                Spacer(grow=True),
                Label("Loading…", tone="hint"),
                Spacer(grow=True),
            ], padding=0, gap=0))
            return

        if self._view == "error":
            ctx.render(Column([
                Spacer(SPACE_LG),
                Card([
                    Label("Error", color=RED, bold=True),
                    Spacer(SPACE_SM),
                    Label(self._error, tone="caption"),
                ]),
            ], padding=SPACE_LG, gap=0))
            return

        assert self._descriptor is not None
        d = self._descriptor
        icon = d.get("icon", "")
        name = d.get("name", "")
        ver = d.get("version", "")
        title = f"{icon}  {name}  v{ver}" if icon else f"{name}  v{ver}"
        desc = d.get("description", "")

        if self._view == "list":
            commands = _commands_at(self._descriptor, self._cmd_path)
            crumb = None
            if self._cmd_path:
                crumb = " › ".join([name] + list(self._cmd_path))
            app_bar = AppBar(title, subtitle=crumb or desc or None)
            app_bar_h = app_bar.measure(ctx.w)
            header_items: list = [app_bar]
            if self._cmd_path:
                back_item = ListItem(title="← Back", selected=False)
                header_items.append(back_item)
                back_h = back_item.measure(ctx.w)
                self._hits = [(app_bar_h, app_bar_h + back_h, "back")]
            else:
                self._hits = []
            header_items.append(self._list.render([
                ListItem(
                    title=cmd["name"] + (" ›" if cmd.get("commands") else ""),
                    subtitle=cmd.get("description") or None,
                    leading=cmd.get("icon") or None,
                    selected=i == self._list.selected_index,
                )
                for i, cmd in enumerate(commands)
            ]))
            footer_hints = [("j/k", "navigate"), ("enter", "open")]
            if self._cmd_path:
                footer_hints.append(("esc", "back"))
            header_items.append(FooterKeys(footer_hints))
            ctx.render(Column(header_items, padding=0, padding_top=0, gap=0))
            return

        if self._view == "form":
            ctx.clear(BG)
            self._render_form(ctx)

    def _render_form(self, ctx: RenderContext) -> None:
        assert self._descriptor is not None
        d = self._descriptor
        icon = d.get("icon", "")
        name = d.get("name", "")
        ver = d.get("version", "")
        title = f"{icon}  {name}  v{ver}" if icon else f"{name}  v{ver}"

        self._hits = []

        # AppBar at top
        app_bar = AppBar(title)
        app_bar_h = app_bar.measure(ctx.w)
        app_bar.render(ctx, 0, 0, ctx.w, app_bar_h)
        y = app_bar_h

        # Back button
        back = ListItem(title="← Back", selected=False)
        back_h = back.measure(ctx.w)
        back.render(ctx, 0, y, ctx.w, back_h)
        self._hits.append((y, y + back_h, "back"))
        y += back_h + SPACE_SM

        if not self._form_fields:
            run = ListItem(title="▶  Run", background=ACCENT, selected=False)
            run_h = run.measure(ctx.w)
            run.render(ctx, 0, y, ctx.w, run_h)
            self._hits.append((y, y + run_h, "run"))
            y += run_h + SPACE_SM
        else:
            for ff in self._form_fields:
                sub = ff.submitted
                if sub is not None:
                    self._field_values[ff.id] = sub
                    if ff.id == self._form_fields[-1].id:
                        self._run()
                fh = ff.measure(ctx.w - 2 * SPACE_LG)
                ff.render(ctx, SPACE_LG, y, ctx.w - 2 * SPACE_LG, fh)
                y += fh

            run = ListItem(title="▶  Run", background=ACCENT, selected=False)
            run_h = run.measure(ctx.w)
            run.render(ctx, 0, y, ctx.w, run_h)
            self._hits.append((y, y + run_h, "run"))
            y += run_h + SPACE_SM

        if self._last_run:
            last_run_h = TEXT_HINT + SPACE_XS * 2 + 4.0
            ctx.rect(SPACE_LG, y, ctx.w - 2 * SPACE_LG, last_run_h, HIGHLIGHT, radius=RADIUS_SM)
            ctx.text(x=SPACE_LG + SPACE_SM, y=y + last_run_h / 2,
                     text=f"$ {self._last_run}",
                     size=TEXT_HINT, color=MUTED, monospace=True,
                     align="left_center", max_width=ctx.w - 2 * SPACE_LG - SPACE_MD, elide=True)

        footer = FooterKeys([("enter", "run"), ("esc", "back")])
        footer_h = footer.measure(ctx.w)
        footer.render(ctx, 0, ctx.h - footer_h, ctx.w, footer_h)

    # ── Interaction ───────────────────────────────────────────────────────────

    def on_click(self, _ctx: RenderContext, _x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        if self._view == "list":
            # Check back button first (tracked in _hits)
            for (yt, yb, tag) in self._hits:
                if yt <= y < yb:
                    self._handle(tag)
                    self.emit.schedule_render(after_ms=16)
                    return
            # Then list items via ListView
            idx = self._list.hit_test(y)
            if idx is not None:
                self._list.set_selected(idx)
                self._selected_idx = idx
                self._handle(idx)
                self.emit.schedule_render(after_ms=16)
            return
        # Form view: use _hits
        for (yt, yb, tag) in self._hits:
            if yt <= y < yb:
                if isinstance(tag, int) and self._view == "list":
                    self._selected_idx = tag
                self._handle(tag)
                self.emit.schedule_render(after_ms=16)
                return

    def _handle(self, tag: object) -> None:
        if tag == "back":
            if self._view == "form":
                self._view = "list"
                self._cmd_path.pop()
            elif self._cmd_path:
                self._cmd_path.pop()
            self._selected_idx = 0
            self._list.set_selected(0)
            return

        if tag == "run":
            self._run()
            return

        if isinstance(tag, int) and self._view == "list":
            assert self._descriptor is not None
            commands = _commands_at(self._descriptor, self._cmd_path)
            if 0 <= tag < len(commands):
                cmd = commands[tag]
                self._cmd_path.append(cmd["name"])
                self._selected_idx = 0
                self._list.set_selected(0)
                if not cmd.get("commands"):
                    self._enter_form(cmd)

    def _enter_form(self, cmd: dict) -> None:
        self._view = "form"
        self._field_values = {}
        self._fields = []
        self._form_fields = []
        for arg in cmd.get("args", []):
            default = arg.get("default")
            self._fields.append({
                "name": arg["name"],
                "type": arg.get("type", "string"),
                "required": arg.get("required", False),
                "placeholder": arg.get("placeholder") or arg.get("description", ""),
                "default_str": "" if default is None else str(default),
            })
            self._form_fields.append(FormField(
                id=arg["name"],
                label=arg["name"],
                placeholder=arg.get("placeholder") or arg.get("description", ""),
                required=arg.get("required", False),
            ))
            if default is not None:
                self._field_values[arg["name"]] = str(default)
        for flag in cmd.get("flags", []):
            default = flag.get("default")
            self._fields.append({
                "name": flag["name"],
                "type": flag.get("type", "string"),
                "required": False,
                "placeholder": flag.get("description", ""),
                "default_str": "" if default is None else str(default),
            })
            self._form_fields.append(FormField(
                id=flag["name"],
                label=flag["name"],
                placeholder=flag.get("description", ""),
                required=False,
            ))
            if default is not None:
                self._field_values[flag["name"]] = str(default)

    def _run(self) -> None:
        assert self._descriptor is not None
        if self._terminal_pane_id == 0:
            self.emit.warn("descriptor-renderer: no linked terminal")
            return
        cli = self._descriptor.get("name", "")
        cmd_str = _build_command(cli, self._cmd_path, self._field_values, self._fields)
        self._last_run = cmd_str
        self.emit.run_in_linked_terminal(self._terminal_pane_id, cmd_str, echo=True)
        self.emit.info(f"descriptor-renderer: ran {cmd_str!r}")

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._view == "list":
            if self._list.handle_key(key):
                self._selected_idx = self._list.selected_index
                self.emit.schedule_render(after_ms=16)
            elif key == "return":
                self._handle(self._list.selected_index)
                self.emit.schedule_render(after_ms=16)
            elif key == "escape" and self._cmd_path:
                self._cmd_path.pop()
                self._selected_idx = 0
                self._list.set_selected(0)
                self.emit.schedule_render(after_ms=16)

        elif self._view == "form":
            if key == "escape":
                self._view = "list"
                self._cmd_path.pop()
                self._selected_idx = 0
                self._list.set_selected(0)
                self.emit.schedule_render(after_ms=16)
            elif key == "return":
                self._run()
                self.emit.schedule_render(after_ms=16)


if __name__ == "__main__":
    DescriptorRendererApp().run()

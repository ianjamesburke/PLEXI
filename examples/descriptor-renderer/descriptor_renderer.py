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
from __future__ import annotations

import json
import sys
import threading
from pathlib import Path

from plexi_sdk import (
    App, RenderContext,
    BG, FG, SURFACE, HIGHLIGHT, ACCENT, MUTED, RED,
    HEADING, BODY, CAPTION, HINT,
    PAD, HEADER_H,
    CapabilityDeniedError,
)

# ── Layout constants ───────────────────────────────────────────────────────────

BTN_H: float = 44.0
BTN_GAP: float = 8.0
FIELD_H: float = 36.0
FIELD_GAP: float = 24.0  # label (16px) + gap below field
ROW_H: float = BTN_H + BTN_GAP


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
        self._scroll_offset: int = 0  # first visible row index (auto-tracked)
        self._list_y0: float = 0.0    # y where list content starts (set each render)
        # Form state
        self._fields: list[dict] = []
        self._field_values: dict[str, str] = {}
        self._last_run: str = ""
        # Hit regions: (y_top, y_bot, tag)
        self._hits: list[tuple[float, float, object]] = []

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
        ctx.rect(0, 0, ctx.w, ctx.h, BG)

        if self._view == "loading":
            ctx.text(x=ctx.w / 2, y=ctx.h / 2, text="Loading…",
                     size=BODY, color=MUTED, align="center")
            return

        if self._view == "error":
            self._render_error(ctx)
            return

        y = self._render_header(ctx)
        y = self._render_breadcrumb(ctx, y)

        if self._view == "list":
            self._list_y0 = y
            self._render_list(ctx, y)
        else:
            self._render_form(ctx, y)

    def _render_header(self, ctx: RenderContext) -> float:
        assert self._descriptor is not None
        h = HEADER_H
        ctx.rect(0, 0, ctx.w, h, SURFACE)
        d = self._descriptor
        icon = d.get("icon", "")
        name = d.get("name", "")
        ver = d.get("version", "")
        label = f"{icon}  {name}  v{ver}" if icon else f"{name}  v{ver}"
        ctx.text(x=PAD, y=h / 2, text=label,
                 size=HEADING, color=FG, bold=True, align="left_center")
        desc = d.get("description", "")
        if desc:
            ctx.text(x=ctx.w - PAD, y=h / 2, text=desc,
                     size=HINT, color=MUTED, align="right_center",
                     max_width=ctx.w * 0.45, elide=True)
        return h + PAD

    def _render_breadcrumb(self, ctx: RenderContext, y: float) -> float:
        assert self._descriptor is not None
        if not self._cmd_path:
            return y
        cli = self._descriptor.get("name", "?")
        crumb = " › ".join([cli] + list(self._cmd_path))
        ctx.text(x=PAD, y=y, text=crumb, size=CAPTION, color=MUTED)
        return y + 22

    def _render_error(self, ctx: RenderContext) -> None:
        ctx.rect(PAD, PAD, ctx.w - PAD * 2, 80.0, SURFACE, radius=6.0)
        ctx.text(x=PAD + 14, y=PAD + 20, text="Error", size=HEADING, color=RED, bold=True)
        ctx.text(x=PAD + 14, y=PAD + 52, text=self._error,
                 size=CAPTION, color=FG, max_width=ctx.w - PAD * 2 - 28, elide=True)

    def _render_list(self, ctx: RenderContext, y0: float) -> None:
        assert self._descriptor is not None
        commands = _commands_at(self._descriptor, self._cmd_path)
        btn_w = ctx.w - PAD * 2
        self._hits = []
        y = y0

        if self._cmd_path:
            ctx.rect(PAD, y, btn_w, BTN_H, SURFACE, radius=6.0)
            ctx.text(x=PAD + 14, y=y + BTN_H / 2, text="← Back",
                     size=BODY, color=ACCENT, bold=True, align="left_center")
            self._hits.append((y, y + BTN_H, "back"))
            y += BTN_H + BTN_GAP

        for i, cmd in enumerate(commands):
            by = y + (i - self._scroll_offset) * ROW_H
            self._hits.append((by, by + BTN_H, i))
            if by + BTN_H <= y0 or by >= ctx.h:
                continue

            selected = (i == self._selected_idx)
            bg = HIGHLIGHT if selected else SURFACE
            name_color = ACCENT if selected else FG
            is_group = bool(cmd.get("commands"))
            icon = cmd.get("icon", "")
            name_label = cmd["name"] + (" ›" if is_group else "")
            display = f"{icon}  {name_label}" if icon else name_label
            desc_text = cmd.get("description", "")

            ctx.rect(PAD, by, btn_w, BTN_H, bg, radius=6.0)
            label_y = by + (BTN_H * 0.38 if desc_text else BTN_H / 2)
            ctx.text(x=PAD + 14, y=label_y, text=display,
                     size=BODY, color=name_color, bold=True, align="left_center")
            if desc_text:
                ctx.text(x=PAD + 14, y=by + BTN_H * 0.72, text=desc_text,
                         size=HINT, color=MUTED, align="left_center",
                         max_width=btn_w - 28, elide=True)

        # Hint only when list overflows visible area
        visible_rows = max(1, int((ctx.h - y) / ROW_H))
        if len(commands) > visible_rows:
            ctx.text(x=ctx.w / 2, y=ctx.h - 16,
                     text=f"j/k  ·  {self._selected_idx + 1} / {len(commands)}",
                     size=HINT, color=MUTED, align="center")

    def _render_form(self, ctx: RenderContext, y0: float) -> None:
        btn_w = ctx.w - PAD * 2
        self._hits = []
        y = y0

        ctx.rect(PAD, y, btn_w, BTN_H, SURFACE, radius=6.0)
        ctx.text(x=PAD + 14, y=y + BTN_H / 2, text="← Back",
                 size=BODY, color=ACCENT, bold=True, align="left_center")
        self._hits.append((y, y + BTN_H, "back"))
        y += BTN_H + BTN_GAP

        if not self._fields:
            ctx.rect(PAD, y, btn_w, BTN_H, ACCENT, radius=6.0)
            ctx.text(x=PAD + btn_w / 2, y=y + BTN_H / 2, text="▶  Run",
                     size=BODY, color=BG, bold=True, align="center")
            self._hits.append((y, y + BTN_H, "run"))
            y += BTN_H + BTN_GAP
        else:
            for field in self._fields:
                name = field["name"]
                req = " *" if field.get("required") else ""
                ctx.text(x=PAD, y=y, text=f"{name}{req}", size=HINT, color=MUTED)
                y += 18
                submitted = ctx.text_input(
                    id=name, x=PAD, y=y, w=btn_w,
                    placeholder=field.get("placeholder", field.get("default_str", "")),
                )
                if submitted is not None:
                    self._field_values[name] = submitted
                    if name == self._fields[-1]["name"]:
                        self._run()
                self._hits.append((y, y + FIELD_H, name))
                y += FIELD_H + FIELD_GAP

            ctx.rect(PAD, y, btn_w, BTN_H, ACCENT, radius=6.0)
            ctx.text(x=PAD + btn_w / 2, y=y + BTN_H / 2, text="▶  Run",
                     size=BODY, color=BG, bold=True, align="center")
            self._hits.append((y, y + BTN_H, "run"))
            y += BTN_H + BTN_GAP

        if self._last_run:
            ctx.rect(PAD, y, btn_w, 34, HIGHLIGHT, radius=4.0)
            ctx.text(x=PAD + 10, y=y + 17, text=f"$ {self._last_run}",
                     size=HINT, color=MUTED, monospace=True,
                     align="left_center", max_width=btn_w - 20, elide=True)

    # ── Interaction ───────────────────────────────────────────────────────────

    def on_click(self, ctx: RenderContext, _x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        for (y_top, y_bot, tag) in self._hits:
            if y_top <= y < y_bot:
                # Sync selection cursor to clicked row so it stays coherent
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
            self._scroll_offset = 0
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
                self._scroll_offset = 0
                if not cmd.get("commands"):
                    self._enter_form(cmd)

    def _enter_form(self, cmd: dict) -> None:
        self._view = "form"
        self._field_values = {}
        self._fields = []
        for arg in cmd.get("args", []):
            default = arg.get("default")
            self._fields.append({
                "name": arg["name"],
                "type": arg.get("type", "string"),
                "required": arg.get("required", False),
                "placeholder": arg.get("placeholder") or arg.get("description", ""),
                "default_str": "" if default is None else str(default),
            })
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

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._view == "list":
            assert self._descriptor is not None
            commands = _commands_at(self._descriptor, self._cmd_path)
            total = len(commands)
            visible_rows = max(1, int((ctx.h - self._list_y0) / ROW_H))

            if key in ("ArrowDown", "j"):
                self._selected_idx = min(self._selected_idx + 1, total - 1)
                self._clamp_scroll(visible_rows)
                self.emit.schedule_render(after_ms=16)
            elif key in ("ArrowUp", "k"):
                self._selected_idx = max(self._selected_idx - 1, 0)
                self._clamp_scroll(visible_rows)
                self.emit.schedule_render(after_ms=16)
            elif key in ("Return", "Enter"):
                self._handle(self._selected_idx)
                self.emit.schedule_render(after_ms=16)
            elif key == "Escape" and self._cmd_path:
                self._cmd_path.pop()
                self._selected_idx = 0
                self._scroll_offset = 0
                self.emit.schedule_render(after_ms=16)

        elif self._view == "form":
            if key == "Escape":
                self._view = "list"
                self._cmd_path.pop()
                self._selected_idx = 0
                self._scroll_offset = 0
                self.emit.schedule_render(after_ms=16)

    def _clamp_scroll(self, visible_rows: int) -> None:
        """Keep _scroll_offset so _selected_idx is always in the visible window."""
        if self._selected_idx < self._scroll_offset:
            self._scroll_offset = self._selected_idx
        elif self._selected_idx >= self._scroll_offset + visible_rows:
            self._scroll_offset = self._selected_idx - visible_rows + 1


if __name__ == "__main__":
    DescriptorRendererApp().run()

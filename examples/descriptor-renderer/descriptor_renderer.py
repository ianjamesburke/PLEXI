#!/usr/bin/env python3
"""descriptor-renderer — auto-UI renderer for --plexi CLI descriptors (issue #361).

Accepts a descriptor path as the first argv or --descriptor <path>.
Requests a linked terminal at startup, then renders the CLI's command
tree as a clickable GUI that drives the terminal.

List view:   top-level commands as buttons; click drills into subgroups
             or opens a form for leaf commands.
Form view:   one text_input per arg/flag; Run button (or Enter on last
             field) builds the shell command and runs it in the linked
             terminal via run_in_linked_terminal.
Keyboard:    Tab/Shift-Tab cycle fields; Enter submits a field value;
             Escape navigates back; j/k or ↑/↓ scroll the command list.
"""
from __future__ import annotations

import json
import sys
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
    return None


def _load_descriptor(path: str) -> dict:
    p = Path(path).expanduser().resolve()
    return json.loads(p.read_text())


def _commands_at(descriptor: dict, path: list[str]) -> list[dict]:
    """Return the commands list at the given path into the descriptor tree."""
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
        is_flag = name.startswith("--")
        if not val:
            continue
        if is_flag and ftype == "bool":
            if val.lower() in ("true", "1", "yes"):
                parts.append(name)
        elif is_flag:
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
        self._scroll: int = 0
        # Form state
        self._fields: list[dict] = []
        self._field_values: dict[str, str] = {}
        self._last_run: str = ""
        # Hit regions updated on each render: list of (y_top, y_bot, tag)
        # tag: int >= 0 = command index; "back" = back button; "run" = run button
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

        try:
            self._terminal_pane_id = self.emit.request_linked_terminal(
                cwd=None,
                label=f"{cli} terminal",
            )
            ctx.status_summary(f"{cli} {ver}  ·  terminal #{self._terminal_pane_id}")
        except CapabilityDeniedError as e:
            self.emit.warn(f"descriptor-renderer: terminal.bindings denied: {e}")
            ctx.status_summary(f"{cli} {ver}  ·  no terminal")

        self._view = "list"
        self.emit.info(f"descriptor-renderer: loaded {path!r}  cli={cli!r}  ver={ver!r}")

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
        box_h = 80.0
        ctx.rect(PAD, PAD, ctx.w - PAD * 2, box_h, SURFACE, radius=6.0)
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

        row_h = BTN_H + BTN_GAP
        offset_y = self._scroll * row_h

        for i, cmd in enumerate(commands):
            by = y + i * row_h - offset_y
            self._hits.append((by, by + BTN_H, i))
            if by + BTN_H <= y0 or by >= ctx.h:
                continue
            is_group = bool(cmd.get("commands"))
            icon = cmd.get("icon", "")
            name_label = cmd["name"] + (" ›" if is_group else "")
            display = f"{icon}  {name_label}" if icon else name_label
            desc_text = cmd.get("description", "")

            ctx.rect(PAD, by, btn_w, BTN_H, SURFACE, radius=6.0)
            label_y = by + (BTN_H * 0.38 if desc_text else BTN_H / 2)
            ctx.text(x=PAD + 14, y=label_y, text=display,
                     size=BODY, color=FG, bold=True, align="left_center")
            if desc_text:
                ctx.text(x=PAD + 14, y=by + BTN_H * 0.72, text=desc_text,
                         size=HINT, color=MUTED, max_width=btn_w - 28, elide=True)

        total_h = len(commands) * row_h
        visible_h = ctx.h - y
        if total_h > visible_h:
            ctx.text(x=ctx.w / 2, y=ctx.h - 16,
                     text=f"↑↓  scroll  ({self._scroll + 1} / {len(commands)})",
                     size=HINT, color=MUTED, align="center")

    def _render_form(self, ctx: RenderContext, y0: float) -> None:
        btn_w = ctx.w - PAD * 2
        self._hits = []
        y = y0

        # Back button
        ctx.rect(PAD, y, btn_w, BTN_H, SURFACE, radius=6.0)
        ctx.text(x=PAD + 14, y=y + BTN_H / 2, text="← Back",
                 size=BODY, color=ACCENT, bold=True, align="left_center")
        self._hits.append((y, y + BTN_H, "back"))
        y += BTN_H + BTN_GAP

        if not self._fields:
            # No args: just a run button
            by = y
            ctx.rect(PAD, by, btn_w, BTN_H, ACCENT, radius=6.0)
            ctx.text(x=PAD + btn_w / 2, y=by + BTN_H / 2, text="▶  Run",
                     size=BODY, color=BG, bold=True, align="center")
            self._hits.append((by, by + BTN_H, "run"))
            y += BTN_H + BTN_GAP
        else:
            for field in self._fields:
                name = field["name"]
                req = " *" if field.get("required") else ""
                ctx.text(x=PAD, y=y, text=f"{name}{req}",
                         size=HINT, color=MUTED)
                y += 18
                submitted = ctx.text_input(
                    id=name,
                    x=PAD, y=y, w=btn_w,
                    placeholder=field.get("placeholder", field.get("default_str", "")),
                )
                if submitted is not None:
                    self._field_values[name] = submitted
                    # Auto-run when last field is submitted
                    if name == self._fields[-1]["name"]:
                        self._run()
                self._hits.append((y, y + FIELD_H, name))
                y += FIELD_H + FIELD_GAP

            by = y
            ctx.rect(PAD, by, btn_w, BTN_H, ACCENT, radius=6.0)
            ctx.text(x=PAD + btn_w / 2, y=by + BTN_H / 2, text="▶  Run",
                     size=BODY, color=BG, bold=True, align="center")
            self._hits.append((by, by + BTN_H, "run"))
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
                self._handle(tag)
                self.emit.schedule_render(after_ms=16)
                return

    def _handle(self, tag: object) -> None:
        if tag == "back":
            if self._view == "form":
                self._view = "list"
                self._cmd_path.pop()
                self._scroll = 0
            elif self._cmd_path:
                self._cmd_path.pop()
                self._scroll = 0
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
                if cmd.get("commands"):
                    self._scroll = 0
                else:
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
                "is_flag": False,
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
                "is_flag": True,
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
            if key in ("ArrowDown", "j"):
                if self._scroll < max(0, total - 1):
                    self._scroll += 1
                    self.emit.schedule_render(after_ms=16)
            elif key in ("ArrowUp", "k"):
                if self._scroll > 0:
                    self._scroll -= 1
                    self.emit.schedule_render(after_ms=16)
            elif key == "Escape" and self._cmd_path:
                self._cmd_path.pop()
                self._scroll = 0
                self.emit.schedule_render(after_ms=16)

        elif self._view == "form":
            if key == "Escape":
                self._view = "list"
                self._cmd_path.pop()
                self._scroll = 0
                self.emit.schedule_render(after_ms=16)


if __name__ == "__main__":
    DescriptorRendererApp().run()

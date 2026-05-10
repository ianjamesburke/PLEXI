#!/usr/bin/env python3
"""mcp-renderer — wraps any stdio MCP server as a navigable tool UI.

Takes the MCP server command as argv:
    mcp_renderer.py npx @modelcontextprotocol/server-filesystem /path

List view:   j/k or ↑/↓ move selection; Enter opens; Escape goes back.
             Selected row is highlighted. Scroll auto-tracks the cursor.
Form view:   host-managed text_input per field; first field auto-focused;
             Tab cycles fields; Enter on last field runs the tool call.
             Escape returns to list.
Result view: Shows tool call output inline. Escape or Back returns to list.
"""

import json
import subprocess
import sys
import threading
from typing import IO

from plexi_sdk import (
    App, RenderContext,
    BG, FG, SURFACE, HIGHLIGHT, ACCENT, MUTED, RED,
    HEADING, BODY, CAPTION, HINT,
    PAD, HEADER_H,
)

# ── Layout constants ───────────────────────────────────────────────────────────

BTN_H: float = 44.0
BTN_GAP: float = 8.0
FIELD_H: float = 36.0
FIELD_GAP: float = 24.0  # label (16px) + gap below field
ROW_H: float = BTN_H + BTN_GAP
RESULT_LINE_H: float = 20.0


# ── MCP stdio protocol ────────────────────────────────────────────────────────

class McpClient:
    """Minimal MCP stdio client — JSON-RPC 2.0 over subprocess stdin/stdout."""

    def __init__(self, cmd: list[str]) -> None:
        self._cmd = cmd
        self._proc: subprocess.Popen | None = None
        self._stdin: IO[bytes] | None = None
        self._stdout: IO[bytes] | None = None
        self._next_id: int = 1
        self._lock = threading.Lock()

    def start(self) -> None:
        self._proc = subprocess.Popen(
            self._cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self._stdin = self._proc.stdin
        self._stdout = self._proc.stdout

    def _send(self, msg: dict) -> None:
        assert self._stdin is not None
        line = json.dumps(msg) + "\n"
        with self._lock:
            self._stdin.write(line.encode())
            self._stdin.flush()

    def _recv(self) -> dict:
        assert self._stdout is not None
        while True:
            raw = self._stdout.readline()
            if not raw:
                raise EOFError("MCP server closed stdout")
            raw = raw.strip()
            if not raw:
                continue
            return json.loads(raw)

    def _call(self, method: str, params: dict) -> dict:
        msg_id = self._next_id
        self._next_id += 1
        self._send({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": params})
        while True:
            resp = self._recv()
            # Skip notifications (no id or id is None)
            if resp.get("id") == msg_id:
                if "error" in resp:
                    raise RuntimeError(f"MCP error: {resp['error']}")
                return resp.get("result", {})

    def initialize(self) -> None:
        self._call("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mcp-renderer", "version": "0.1.0"},
        })
        # Send initialized notification (no id, no response expected)
        self._send({"jsonrpc": "2.0", "id": None, "method": "notifications/initialized", "params": {}})

    def list_tools(self) -> list[dict]:
        result = self._call("tools/list", {})
        return result.get("tools", [])

    def call_tool(self, name: str, arguments: dict) -> list[dict]:
        result = self._call("tools/call", {"name": name, "arguments": arguments})
        return result.get("content", [])

    def close(self) -> None:
        if self._proc is not None:
            try:
                self._proc.terminate()
            except Exception:
                pass


# ── App ────────────────────────────────────────────────────────────────────────

class McpRendererApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self._view: str = "loading"   # loading | error | list | form | result
        self._error: str = ""
        self._tools: list[dict] = []
        self._selected_idx: int = 0
        self._scroll_offset: int = 0
        self._list_y0: float = 0.0
        # Form state
        self._active_tool: dict = {}
        self._fields: list[dict] = []
        self._field_values: dict[str, str] = {}
        # Result state
        self._result_lines: list[str] = []
        self._result_scroll: int = 0
        self._calling: bool = False
        # Hit regions: (y_top, y_bot, tag)
        self._hits: list[tuple[float, float, object]] = []
        # MCP client
        self._client: McpClient | None = None
        self._tools_ready = threading.Event()

        argv = sys.argv[1:]
        if not argv:
            self._error = "Usage: mcp_renderer.py <command> [args...]\nExample: mcp_renderer.py npx -y @modelcontextprotocol/server-filesystem /tmp"
            self._view = "error"
            ctx.status_summary("mcp-renderer: no command")
            return

        ctx.status_summary("mcp-renderer: connecting…")
        self.emit.info(f"mcp-renderer: spawning {argv!r}")
        threading.Thread(target=self._connect, args=(argv,), daemon=True).start()

    def _connect(self, cmd: list[str]) -> None:
        try:
            client = McpClient(cmd)
            client.start()
            client.initialize()
            tools = client.list_tools()
            self._client = client
            self._tools = tools
            self._view = "list"
            self.emit.info(f"mcp-renderer: got {len(tools)} tools")
        except Exception as e:
            self._error = f"Failed to connect to MCP server:\n{e}"
            self._view = "error"
            self.emit.warn(f"mcp-renderer: connect failed: {e}")
        self._tools_ready.set()
        self.emit.schedule_render(after_ms=16)

    def _call_tool_bg(self, name: str, arguments: dict) -> None:
        assert self._client is not None
        try:
            content = self._client.call_tool(name, arguments)
            lines: list[str] = []
            for item in content:
                t = item.get("type", "")
                if t == "text":
                    lines.extend(item.get("text", "").splitlines())
                elif t == "image":
                    lines.append("[Image — open in terminal to view]")
                else:
                    lines.append(json.dumps(item))
            self._result_lines = lines
            self.emit.info(f"mcp-renderer: tool {name!r} returned {len(lines)} lines")
        except Exception as e:
            self._result_lines = [f"Error: {e}"]
            self.emit.warn(f"mcp-renderer: tool call failed: {e}")
        finally:
            self._calling = False
            self._view = "result"
            self._result_scroll = 0
        self.emit.schedule_render(after_ms=16)

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, BG)

        if self._view == "loading":
            ctx.text(x=ctx.w / 2, y=ctx.h / 2, text="Connecting to MCP server…",
                     size=BODY, color=MUTED, align="center")
            return

        if self._view == "error":
            self._render_error(ctx)
            return

        y = self._render_header(ctx)

        if self._view == "list":
            self._list_y0 = y
            self._render_list(ctx, y)
        elif self._view == "form":
            self._render_form(ctx, y)
        elif self._view == "result":
            self._render_result(ctx, y)

    def _render_header(self, ctx: RenderContext) -> float:
        h = HEADER_H
        ctx.rect(0, 0, ctx.w, h, SURFACE)
        cmd_name = sys.argv[1] if len(sys.argv) > 1 else "mcp"
        ctx.text(x=PAD, y=h / 2, text=f"MCP · {cmd_name}",
                 size=HEADING, color=FG, bold=True, align="left_center")
        tool_count = len(self._tools)
        if tool_count:
            ctx.text(x=ctx.w - PAD, y=h / 2,
                     text=f"{tool_count} tool{'s' if tool_count != 1 else ''}",
                     size=HINT, color=MUTED, align="right_center")
        return h + PAD

    def _render_error(self, ctx: RenderContext) -> None:
        box_h = 100.0
        ctx.rect(PAD, PAD, ctx.w - PAD * 2, box_h, SURFACE, radius=6.0)
        ctx.text(x=PAD + 14, y=PAD + 22, text="Error", size=HEADING, color=RED, bold=True)
        for i, line in enumerate(self._error.splitlines()):
            ctx.text(x=PAD + 14, y=PAD + 48 + i * 20, text=line,
                     size=CAPTION, color=FG, max_width=ctx.w - PAD * 2 - 28, elide=True)

    def _render_list(self, ctx: RenderContext, y0: float) -> None:
        btn_w = ctx.w - PAD * 2
        self._hits = []
        y = y0

        if not self._tools:
            ctx.text(x=ctx.w / 2, y=ctx.h / 2, text="No tools found",
                     size=BODY, color=MUTED, align="center")
            return

        for i, tool in enumerate(self._tools):
            by = y + (i - self._scroll_offset) * ROW_H
            self._hits.append((by, by + BTN_H, i))
            if by + BTN_H <= y0 or by >= ctx.h:
                continue

            selected = (i == self._selected_idx)
            bg = HIGHLIGHT if selected else SURFACE
            name_color = ACCENT if selected else FG
            name = tool.get("name", "?")
            desc = tool.get("description", "")

            ctx.rect(PAD, by, btn_w, BTN_H, bg, radius=6.0)
            label_y = by + (BTN_H * 0.38 if desc else BTN_H / 2)
            ctx.text(x=PAD + 14, y=label_y, text=name,
                     size=BODY, color=name_color, bold=True, align="left_center")
            if desc:
                ctx.text(x=PAD + 14, y=by + BTN_H * 0.72, text=desc,
                         size=HINT, color=MUTED, align="left_center",
                         max_width=btn_w - 28, elide=True)

        visible_rows = max(1, int((ctx.h - y0) / ROW_H))
        if len(self._tools) > visible_rows:
            ctx.text(x=ctx.w / 2, y=ctx.h - 16,
                     text=f"j/k  ·  {self._selected_idx + 1} / {len(self._tools)}",
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

        # Tool name
        tool_name = self._active_tool.get("name", "")
        ctx.text(x=PAD, y=y, text=tool_name, size=HEADING, color=FG, bold=True)
        y += 28

        tool_desc = self._active_tool.get("description", "")
        if tool_desc:
            ctx.text(x=PAD, y=y, text=tool_desc, size=CAPTION, color=MUTED,
                     max_width=btn_w, elide=False)
            y += 22

        y += 8

        if not self._fields:
            # No args needed — just a Run button
            ctx.rect(PAD, y, btn_w, BTN_H, ACCENT, radius=6.0)
            ctx.text(x=PAD + btn_w / 2, y=y + BTN_H / 2, text="▶  Call Tool",
                     size=BODY, color=BG, bold=True, align="center")
            self._hits.append((y, y + BTN_H, "run"))
        else:
            for field in self._fields:
                name = field["name"]
                req = " *" if field.get("required") else ""
                ctx.text(x=PAD, y=y, text=f"{name}{req}", size=HINT, color=MUTED)
                y += 18
                submitted = ctx.text_input(
                    id=name, x=PAD, y=y, w=btn_w,
                    placeholder=field.get("placeholder", ""),
                )
                if submitted is not None:
                    self._field_values[name] = submitted
                    if name == self._fields[-1]["name"]:
                        self._run_tool()
                self._hits.append((y, y + FIELD_H, name))
                y += FIELD_H + FIELD_GAP

            if self._calling:
                ctx.rect(PAD, y, btn_w, BTN_H, SURFACE, radius=6.0)
                ctx.text(x=PAD + btn_w / 2, y=y + BTN_H / 2, text="Calling…",
                         size=BODY, color=MUTED, align="center")
            else:
                ctx.rect(PAD, y, btn_w, BTN_H, ACCENT, radius=6.0)
                ctx.text(x=PAD + btn_w / 2, y=y + BTN_H / 2, text="▶  Call Tool",
                         size=BODY, color=BG, bold=True, align="center")
                self._hits.append((y, y + BTN_H, "run"))

    def _render_result(self, ctx: RenderContext, y0: float) -> None:
        btn_w = ctx.w - PAD * 2
        self._hits = []
        y = y0

        # Back button
        ctx.rect(PAD, y, btn_w, BTN_H, SURFACE, radius=6.0)
        ctx.text(x=PAD + 14, y=y + BTN_H / 2, text="← Back to Tools",
                 size=BODY, color=ACCENT, bold=True, align="left_center")
        self._hits.append((y, y + BTN_H, "back"))
        y += BTN_H + BTN_GAP

        # Result label
        tool_name = self._active_tool.get("name", "result")
        ctx.text(x=PAD, y=y, text=f"Result: {tool_name}", size=HEADING, color=FG, bold=True)
        y += 28

        # Scrollable result lines
        result_area_h = ctx.h - y - PAD
        visible_lines = max(1, int(result_area_h / RESULT_LINE_H))
        lines = self._result_lines
        total = len(lines)

        for i in range(visible_lines):
            line_idx = i + self._result_scroll
            if line_idx >= total:
                break
            line = lines[line_idx]
            ctx.text(x=PAD, y=y + i * RESULT_LINE_H, text=line,
                     size=CAPTION, color=FG, monospace=True,
                     max_width=btn_w, elide=True)

        if total > visible_lines:
            ctx.text(x=ctx.w / 2, y=ctx.h - 16,
                     text=f"↑/↓  ·  line {self._result_scroll + 1} / {total}",
                     size=HINT, color=MUTED, align="center")

    # ── Interaction ───────────────────────────────────────────────────────────

    def on_click(self, _ctx: RenderContext, _x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        for (y_top, y_bot, tag) in self._hits:
            if y_top <= y < y_bot:
                if isinstance(tag, int) and self._view == "list":
                    self._selected_idx = tag
                self._handle(tag)
                self.emit.schedule_render(after_ms=16)
                return

    def _handle(self, tag: object) -> None:
        if tag == "back":
            if self._view in ("form", "result"):
                self._view = "list"
                self._selected_idx = 0
                self._scroll_offset = 0
            return

        if tag == "run":
            self._run_tool()
            return

        if isinstance(tag, int) and self._view == "list":
            if 0 <= tag < len(self._tools):
                self._enter_form(self._tools[tag])

    def _enter_form(self, tool: dict) -> None:
        self._active_tool = tool
        self._field_values = {}
        self._fields = []
        schema = tool.get("inputSchema", {})
        props = schema.get("properties", {})
        required_set = set(schema.get("required", []))
        for prop_name, prop_schema in props.items():
            self._fields.append({
                "name": prop_name,
                "type": prop_schema.get("type", "string"),
                "required": prop_name in required_set,
                "placeholder": prop_schema.get("description", prop_schema.get("title", "")),
            })
        self._view = "form"

    def _run_tool(self) -> None:
        if self._calling or self._client is None:
            return
        self._calling = True
        arguments: dict = {}
        for field in self._fields:
            val = self._field_values.get(field["name"], "").strip()
            if val:
                # Coerce to int/float if schema says so
                ftype = field.get("type", "string")
                if ftype == "integer":
                    try:
                        arguments[field["name"]] = int(val)
                    except ValueError:
                        arguments[field["name"]] = val
                elif ftype == "number":
                    try:
                        arguments[field["name"]] = float(val)
                    except ValueError:
                        arguments[field["name"]] = val
                elif ftype == "boolean":
                    arguments[field["name"]] = val.lower() in ("true", "1", "yes")
                else:
                    arguments[field["name"]] = val
        tool_name = self._active_tool.get("name", "")
        self.emit.info(f"mcp-renderer: calling tool {tool_name!r} with {arguments!r}")
        threading.Thread(
            target=self._call_tool_bg,
            args=(tool_name, arguments),
            daemon=True,
        ).start()

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._view == "list":
            total = len(self._tools)
            visible_rows = max(1, int((ctx.h - self._list_y0) / ROW_H))

            if key in ("down", "j"):
                self._selected_idx = min(self._selected_idx + 1, total - 1)
                self._clamp_scroll(visible_rows)
                self.emit.schedule_render(after_ms=16)
            elif key in ("up", "k"):
                self._selected_idx = max(self._selected_idx - 1, 0)
                self._clamp_scroll(visible_rows)
                self.emit.schedule_render(after_ms=16)
            elif key in ("Return", "Enter"):
                self._handle(self._selected_idx)
                self.emit.schedule_render(after_ms=16)

        elif self._view == "form":
            if key == "Escape":
                self._view = "list"
                self._selected_idx = 0
                self._scroll_offset = 0
                self.emit.schedule_render(after_ms=16)

        elif self._view == "result":
            total = len(self._result_lines)
            if key in ("down", "j"):
                self._result_scroll = min(self._result_scroll + 1, max(0, total - 1))
                self.emit.schedule_render(after_ms=16)
            elif key in ("up", "k"):
                self._result_scroll = max(self._result_scroll - 1, 0)
                self.emit.schedule_render(after_ms=16)
            elif key == "Escape":
                self._view = "list"
                self._selected_idx = 0
                self._scroll_offset = 0
                self.emit.schedule_render(after_ms=16)

    def _clamp_scroll(self, visible_rows: int) -> None:
        if self._selected_idx < self._scroll_offset:
            self._scroll_offset = self._selected_idx
        elif self._selected_idx >= self._scroll_offset + visible_rows:
            self._scroll_offset = self._selected_idx - visible_rows + 1


if __name__ == "__main__":
    McpRendererApp().run()

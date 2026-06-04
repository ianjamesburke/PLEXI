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

import argparse
import asyncio
import json
import os
import select
import subprocess
import threading
from typing import IO

from plexi_sdk import App, RenderContext, Arg
from plexi_sdk.ui import (
    Column, AppBar, FormField, Scrollable, FooterKeys,
    ListItem, Label, Spacer, Card,
    BG, ACCENT, RED,
    SPACE_XS, SPACE_SM, SPACE_LG,
)
from plexi_sdk.widgets import ListView


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
        env = os.environ.copy()
        extra = "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin"
        env["PATH"] = extra + ":" + env.get("PATH", "")
        self._proc = subprocess.Popen(
            self._cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )
        self._stdin = self._proc.stdin
        self._stdout = self._proc.stdout
        # Drain stderr in background so it doesn't block; last line kept for diagnostics.
        self._last_stderr: str = ""
        stderr_pipe = self._proc.stderr
        def _drain_stderr() -> None:
            assert stderr_pipe is not None
            for line in stderr_pipe:
                self._last_stderr = line.decode(errors="replace").rstrip()
        threading.Thread(target=_drain_stderr, daemon=True).start()

    def _send(self, msg: dict) -> None:
        assert self._stdin is not None
        line = json.dumps(msg) + "\n"
        with self._lock:
            self._stdin.write(line.encode())
            self._stdin.flush()

    def _recv(self, timeout: float = 30.0) -> dict:
        assert self._stdout is not None
        while True:
            ready, _, _ = select.select([self._stdout], [], [], timeout)
            if not ready:
                raise TimeoutError(f"MCP server did not respond within {timeout:.0f}s")
            raw = self._stdout.readline(65536)
            if not raw:
                raise EOFError("MCP server closed stdout")
            raw = raw.strip()
            if not raw:
                continue
            return json.loads(raw)

    def _call(self, method: str, params: dict, timeout: float = 30.0) -> dict:
        msg_id = self._next_id
        self._next_id += 1
        self._send({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": params})
        while True:
            resp = self._recv(timeout=timeout)
            # Skip notifications (no id or id is None)
            if resp.get("id") == msg_id:
                if "error" in resp:
                    raise RuntimeError(f"MCP error: {resp['error']}")
                return resp.get("result", {})

    def initialize(self, timeout: float = 10.0) -> None:
        self._call("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mcp-renderer", "version": "0.1.0"},
        }, timeout=timeout)
        # Send initialized notification (no id, no response expected)
        self._send({"jsonrpc": "2.0", "id": None, "method": "notifications/initialized", "params": {}})

    def list_tools(self, timeout: float = 30.0) -> list[dict]:
        result = self._call("tools/list", {}, timeout=timeout)
        return result.get("tools", [])

    def call_tool(self, name: str, arguments: dict) -> list[dict]:
        result = self._call("tools/call", {"name": name, "arguments": arguments})
        return result.get("content", [])

    def close(self) -> None:
        if self._proc is not None:
            try:
                self._proc.terminate()
                self._proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._proc.kill()
            except Exception:
                pass


# ── App ────────────────────────────────────────────────────────────────────────

class McpRendererApp(App):
    cmd: Arg[list] = Arg(positional=True, nargs=argparse.REMAINDER, default=[])

    def on_init(self, ctx: RenderContext) -> None:
        self._view: str = "loading"   # loading | error | list | form | result
        self._error: str = ""
        self._tools: list[dict] = []
        self._selected_idx: int = 0
        # Form state
        self._active_tool: dict = {}
        self._fields: list[dict] = []
        self._field_values: dict[str, str] = {}
        self._form_fields: list[FormField] = []
        # Result state
        self._result_lines: list[str] = []
        self._calling: bool = False  # True while tool call is in-flight; guards against concurrent calls
        # Hit regions for form/result views: (y_top, y_bot, tag)
        self._hits: list[tuple[float, float, object]] = []
        # UI components
        self._list = ListView(item_height=ListItem.HEIGHT_DOUBLE)
        self._result_scrollable = Scrollable(Label("", tone="hint"))
        # MCP client
        self._client: McpClient | None = None
        self._tools_ready = threading.Event()

        if not self.cmd:
            self._error = "Usage: plexi open mcp-renderer <command> [args...]\nExample: plexi open mcp-renderer npx -y @modelcontextprotocol/server-filesystem /tmp"
            self._view = "error"
            ctx.status_summary("mcp-renderer: no command")
            return

        ctx.status_summary("mcp-renderer: connecting…")
        self.emit.info(f"mcp-renderer: spawning {self.cmd!r}")
        try:
            threading.Thread(target=self._connect, args=(self.cmd,), daemon=True).start()
        except RuntimeError as e:
            self._error = f"Failed to start connection thread: {e}"
            self._view = "error"
            self.emit.warn(f"mcp-renderer: thread spawn failed: {e}")

    def _connect(self, cmd: list[str]) -> None:
        client = McpClient(cmd)
        try:
            client.start()
            client.initialize(timeout=10.0)
            tools = client.list_tools(timeout=10.0)
            self._client = client
            self._tools = tools
            self._view = "list"
            self.emit.info(f"mcp-renderer: got {len(tools)} tools")
        except Exception as e:
            stderr = getattr(client, "_last_stderr", "")
            detail = f"\nServer stderr: {stderr}" if stderr else ""
            self._error = f"Failed to connect to MCP server:\n{e}{detail}"
            self._view = "error"
            self.emit.warn(f"mcp-renderer: connect failed: {e}{detail}")
        self._tools_ready.set()
        self.emit.schedule_render(after_ms=16)

    def _do_call_tool(self, name: str, arguments: dict) -> list[str]:
        """Blocking MCP call — runs via asyncio.to_thread. Returns result lines."""
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
            self.emit.info(f"mcp-renderer: tool {name!r} returned {len(lines)} lines")
            return lines
        except Exception as e:
            self.emit.warn(f"mcp-renderer: tool call failed: {e}")
            return [f"Error: {e}"]

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        cmd_name = self.cmd[0] if self.cmd else "mcp"
        tool_count = len(self._tools)
        subtitle = f"{tool_count} tool{'s' if tool_count != 1 else ''}" if tool_count else None

        if self._view == "loading":
            ctx.render(Column([
                AppBar(f"MCP · {cmd_name}"),
                Spacer(grow=True),
                Label("Connecting to MCP server…", tone="hint"),
                Spacer(grow=True),
            ], padding=0, padding_top=0, gap=0))
            return

        if self._view == "error":
            ctx.render(Column([
                AppBar(f"MCP · {cmd_name}"),
                Spacer(SPACE_LG),
                Card([
                    Label("Error", color=RED, bold=True),
                    Spacer(SPACE_SM),
                    Label(self._error, tone="caption"),
                ]),
            ], padding=SPACE_LG, gap=0))
            return

        if self._view == "list":
            ctx.render(Column([
                AppBar(f"MCP · {cmd_name}", subtitle=subtitle),
                self._list.render([
                    ListItem(
                        title=t["name"],
                        subtitle=t.get("description") or None,
                        selected=i == self._list.selected_index,
                    )
                    for i, t in enumerate(self._tools)
                ]),
            ], padding=0, padding_top=0, gap=0))
            return

        if self._view == "form":
            ctx.clear(BG)
            self._render_form(ctx)
            return

        if self._view == "result":
            tool_name = self._active_tool.get("name", "result")
            if self._result_lines:
                result_content = Column(
                    [Label(line, tone="caption") for line in self._result_lines],
                    padding=SPACE_LG, padding_top=SPACE_SM, gap=SPACE_XS,
                )
            else:
                result_content = Column([Label("No output", tone="hint")], padding=SPACE_LG)
            self._result_scrollable.child = result_content
            ctx.render(Column([
                AppBar(f"Result: {tool_name}"),
                self._result_scrollable,
                FooterKeys([("j/k", "scroll"), ("esc", "back")]),
            ], padding=0, padding_top=0, gap=0))

    def _render_form(self, ctx: RenderContext) -> None:
        self._hits = []

        # AppBar at top
        app_bar = AppBar(self._active_tool.get("name", ""),
                         subtitle=self._active_tool.get("description") or None)
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
            # No-arg tool — just show run button
            run = ListItem(title="▶  Call Tool", background=ACCENT, selected=False)
            run_h = run.measure(ctx.w)
            run.render(ctx, 0, y, ctx.w, run_h)
            self._hits.append((y, y + run_h, "run"))
        else:
            # Render form fields; submissions are handled in on_text_submitted
            for ff in self._form_fields:
                fh = ff.measure(ctx.w - 2 * SPACE_LG)
                ff.render(ctx, SPACE_LG, y, ctx.w - 2 * SPACE_LG, fh)
                y += fh

            if self._calling:
                calling = Label("Calling…", tone="hint")
                calling_h = calling.measure(ctx.w - 2 * SPACE_LG)
                calling.render(ctx, SPACE_LG, y, ctx.w - 2 * SPACE_LG, calling_h)
            else:
                run = ListItem(title="▶  Call Tool", background=ACCENT, selected=False)
                run_h = run.measure(ctx.w)
                run.render(ctx, 0, y, ctx.w, run_h)
                self._hits.append((y, y + run_h, "run"))

    # ── Event handlers ────────────────────────────────────────────────────────

    async def on_text_submitted(self, ctx: RenderContext, id: str, text: str) -> None:
        """Handle TextInput submissions — field values and tool dispatch live here."""
        if self._view != "form":
            return
        self._field_values[id] = text
        self.emit.info(f"mcp-renderer: field {id!r} submitted: {text!r}")
        # If this was the last field, run the tool automatically
        if self._form_fields and id == self._form_fields[-1].id:
            await self._run_tool()
        else:
            self.emit.schedule_render(after_ms=16)

    async def on_click(self, _ctx: RenderContext, _x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        # List view: use ListView hit detection
        if self._view == "list":
            idx = self._list.hit_test(y)
            if idx is not None:
                self._list.set_selected(idx)
                self._selected_idx = idx
                await self._handle(idx)
                self.emit.schedule_render(after_ms=16)
            return
        # Form/result views: use _hits
        for (y_top, y_bot, tag) in self._hits:
            if y_top <= y < y_bot:
                await self._handle(tag)
                self.emit.schedule_render(after_ms=16)
                return

    async def _handle(self, tag: object) -> None:
        if tag == "back":
            if self._view in ("form", "result"):
                self._view = "list"
                self._selected_idx = 0
                self._list.set_selected(0)
            return

        if tag == "run":
            await self._run_tool()
            return

        if isinstance(tag, int) and self._view == "list":
            if 0 <= tag < len(self._tools):
                self._enter_form(self._tools[tag])

    def _enter_form(self, tool: dict) -> None:
        self._active_tool = tool
        self._field_values = {}
        self._fields = []
        self._form_fields = []
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
            self._form_fields.append(FormField(
                id=prop_name,
                label=prop_name,
                placeholder=prop_schema.get("description", prop_schema.get("title", "")),
                required=prop_name in required_set,
            ))
        self._view = "form"

    async def _run_tool(self) -> None:
        if self._calling or self._client is None:
            return
        self._calling = True
        arguments: dict = {}
        for field in self._fields:
            val = self._field_values.get(field["name"], "").strip()
            if val:
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
        self.emit.schedule_render(after_ms=16)  # show "Calling…" state
        result_lines = await asyncio.to_thread(self._do_call_tool, tool_name, arguments)
        self._calling = False
        self._result_lines = result_lines
        self._view = "result"
        self._result_scrollable.scroll_offset = 0.0
        self.emit.schedule_render(after_ms=16)

    async def on_escape(self, _ctx):
        if self._view in ("form", "result"):
            await self._handle("back")
            self.emit.schedule_render(after_ms=16)
            return True
        return False

    async def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._view == "list":
            if self._list.handle_key(key):
                self._selected_idx = self._list.selected_index
                self.emit.schedule_render(after_ms=16)
            elif key == "return":
                await self._handle(self._list.selected_index)
                self.emit.schedule_render(after_ms=16)

        elif self._view == "form":
            if key == "return":
                await self._handle("run")

        elif self._view == "result":
            if self._result_scrollable.handle_key(key):
                self.emit.schedule_render(after_ms=16)


if __name__ == "__main__":
    McpRendererApp().run()

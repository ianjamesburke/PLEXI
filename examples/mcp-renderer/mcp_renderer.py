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
import os
import select
import subprocess
import sys
import threading
from typing import IO

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, AppBar, SelectList, FormField, ScrollLog, FooterKeys,
    ListItem, Label, Spacer, Card,
    ACCENT, RED,
    TEXT_CAPTION,
    SPACE_SM, SPACE_LG,
)


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
                self._proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._proc.kill()
            except Exception:
                pass


# ── App ────────────────────────────────────────────────────────────────────────

class McpRendererApp(App):

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
        self._calling: bool = False
        # Hit regions: (y_top, y_bot, tag)
        self._hits: list[tuple[float, float, object]] = []
        # UI components
        self._select_list = SelectList([])
        self._result_log = ScrollLog([], line_size=TEXT_CAPTION)
        # MCP client
        self._client: McpClient | None = None
        self._tools_ready = threading.Event()

        argv = sys.argv[1:]
        if not argv:
            self._error = "Usage: plexi open mcp-renderer <command> [args...]\nExample: plexi open mcp-renderer npx -y @modelcontextprotocol/server-filesystem /tmp"
            self._view = "error"
            ctx.status_summary("mcp-renderer: no command")
            return

        ctx.status_summary("mcp-renderer: connecting…")
        self.emit.info(f"mcp-renderer: spawning {argv!r}")
        try:
            threading.Thread(target=self._connect, args=(argv,), daemon=True).start()
        except RuntimeError as e:
            self._error = f"Failed to start connection thread: {e}"
            self._view = "error"
            self.emit.warn(f"mcp-renderer: thread spawn failed: {e}")

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
        self.emit.schedule_render(after_ms=16)

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        cmd_name = sys.argv[1] if len(sys.argv) > 1 else "mcp"
        tool_count = len(self._tools)
        subtitle = f"{tool_count} tool{'s' if tool_count != 1 else ''}" if tool_count else None

        if self._view == "loading":
            ctx.render(Column([
                AppBar(f"MCP · {cmd_name}"),
                Spacer(grow=True),
                Label("Connecting to MCP server…", tone="hint"),
                Spacer(grow=True),
            ], padding=0, gap=0))
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
            self._select_list.items = [
                {"name": t["name"], "description": t.get("description") or None}
                for t in self._tools
            ]
            ctx.render(Column([
                AppBar(f"MCP · {cmd_name}", subtitle=subtitle),
                self._select_list,
            ], padding=0, gap=0))
            return

        if self._view == "form":
            self._render_form(ctx)
            return

        if self._view == "result":
            self._result_log.lines = self._result_lines
            tool_name = self._active_tool.get("name", "result")
            ctx.render(Column([
                AppBar(f"Result: {tool_name}"),
                self._result_log,
                FooterKeys([("↑/↓", "scroll"), ("esc", "back")]),
            ], padding=0, gap=0))

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
            # Process submitted values from form fields
            for ff in self._form_fields:
                sub = ff.submitted
                if sub is not None:
                    self._field_values[ff.id] = sub
                    if ff.id == self._form_fields[-1].id:
                        self._run_tool()
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

    # ── Interaction ───────────────────────────────────────────────────────────

    def on_click(self, _ctx: RenderContext, _x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        # List view: use SelectList hit detection
        if self._view == "list":
            idx = self._select_list.hit_index(y)
            if idx is not None:
                self._selected_idx = idx
                self._handle(idx)
                self.emit.schedule_render(after_ms=16)
            return
        # Form/result views: use _hits
        for (y_top, y_bot, tag) in self._hits:
            if y_top <= y < y_bot:
                self._handle(tag)
                self.emit.schedule_render(after_ms=16)
                return

    def _handle(self, tag: object) -> None:
        if tag == "back":
            if self._view in ("form", "result"):
                self._view = "list"
                self._selected_idx = 0
                self._select_list.selected_idx = 0
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

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._view == "list":
            if self._select_list.handle_key(key):
                self._selected_idx = self._select_list.selected_idx
                self.emit.schedule_render(after_ms=16)
            elif key in ("Return", "Enter"):
                self._handle(self._select_list.selected_idx)
                self.emit.schedule_render(after_ms=16)

        elif self._view == "form":
            if key == "Escape":
                self._view = "list"
                self._selected_idx = 0
                self._select_list.selected_idx = 0
                self.emit.schedule_render(after_ms=16)

        elif self._view == "result":
            if key == "Escape":
                self._view = "list"
                self._select_list.selected_idx = 0
                self.emit.schedule_render(after_ms=16)


if __name__ == "__main__":
    McpRendererApp().run()

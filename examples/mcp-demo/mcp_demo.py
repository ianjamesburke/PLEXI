#!/usr/bin/env python3
"""mcp-demo — a note-taking app that exposes tools via [app.mcp] (#958).

Tools exposed:
  add_note(text)   — append a note
  list_notes()     — return all notes
  clear_notes()    — delete all notes

Connect Claude Desktop by adding to claude_desktop_config.json:
  { "mcpServers": { "mcp-demo": { "url": "http://localhost:<PLEXI_MCP_PORT>/mcp" } } }
"""

import os
from plexi_sdk import App, RenderContext, BG, FG, SURFACE, ACCENT, MUTED, HEADING, BODY, CAPTION, HINT, PAD, HEADER_H

BTN_H = 44.0
FIELD_H = 36.0
ROW_H = BTN_H + 8.0


class McpDemoApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._notes: list[str] = []
        self._mcp_port: str = os.environ.get("PLEXI_MCP_PORT", "not set")
        self.emit.info(f"mcp-demo: started, PLEXI_MCP_PORT={self._mcp_port}")
        ctx.status_summary(f"MCP Demo  ·  port {self._mcp_port}")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, BG)
        y = self._render_header(ctx)
        y = self._render_mcp_info(ctx, y)
        y = self._render_input(ctx, y)
        self._render_notes(ctx, y)

    def _render_header(self, ctx: RenderContext) -> float:
        ctx.rect(0, 0, ctx.w, HEADER_H, SURFACE)
        ctx.text(x=PAD, y=HEADER_H/2, text="MCP Demo Notes", size=HEADING, color=FG, bold=True, align="left_center")
        ctx.text(x=ctx.w-PAD, y=HEADER_H/2, text=f"{len(self._notes)} notes", size=HINT, color=MUTED, align="right_center")
        return HEADER_H + PAD

    def _render_mcp_info(self, ctx: RenderContext, y: float) -> float:
        ctx.rect(PAD, y, ctx.w - PAD*2, 56, SURFACE, radius=6.0)
        ctx.text(x=PAD+12, y=y+14, text="MCP endpoint", size=HINT, color=MUTED)
        url = f"http://localhost:{self._mcp_port}/mcp" if self._mcp_port != "not set" else "(launch via Plexi to get port)"
        ctx.text(x=PAD+12, y=y+38, text=url, size=CAPTION, color=ACCENT, monospace=True)
        return y + 56 + 12

    def _render_input(self, ctx: RenderContext, y: float) -> float:
        btn_w = ctx.w - PAD*2
        ctx.text(x=PAD, y=y, text="New note *", size=HINT, color=MUTED)
        y += 18
        submitted = ctx.text_input(id="draft", x=PAD, y=y, w=btn_w, placeholder="Type a note…")
        if submitted is not None:
            self._add_note(submitted)
        y += FIELD_H + 8
        ctx.rect(PAD, y, btn_w, BTN_H, ACCENT, radius=6.0)
        ctx.text(x=PAD + btn_w/2, y=y+BTN_H/2, text="Add Note", size=BODY, color=BG, bold=True, align="center")
        return y + BTN_H + 16

    def _render_notes(self, ctx: RenderContext, y0: float) -> None:
        btn_w = ctx.w - PAD*2
        if not self._notes:
            ctx.text(x=ctx.w/2, y=y0+40, text="No notes yet", size=CAPTION, color=MUTED, align="center")
            return
        y = y0
        for note in reversed(self._notes):
            ctx.rect(PAD, y, btn_w, BTN_H, SURFACE, radius=6.0)
            ctx.text(x=PAD+12, y=y+BTN_H/2, text=note, size=BODY, color=FG, align="left_center",
                     max_width=btn_w-24, elide=True)
            y += ROW_H
            if y > ctx.h:
                break

    def _add_note(self, text: str) -> None:
        text = text.strip()
        if text:
            self._notes.append(text)
            self.emit.info(f"mcp-demo: add_note text={text!r}")
            self.emit.schedule_render(after_ms=16)

    def on_mcp_call(self, _ctx: RenderContext, tool_name: str, arguments: dict) -> dict:
        self.emit.info(f"mcp-demo: on_mcp_call tool={tool_name!r} args={arguments!r}")
        if tool_name == "add_note":
            text = arguments.get("text", "").strip()
            if not text:
                return {"content": [{"type": "text", "text": "error: text is required"}]}
            self._notes.append(text)
            self.emit.schedule_render(after_ms=16)
            return {"content": [{"type": "text", "text": f"Added note: {text!r}"}]}
        elif tool_name == "list_notes":
            if not self._notes:
                return {"content": [{"type": "text", "text": "No notes."}]}
            lines = "\n".join(f"{i+1}. {n}" for i, n in enumerate(self._notes))
            return {"content": [{"type": "text", "text": lines}]}
        elif tool_name == "clear_notes":
            count = len(self._notes)
            self._notes.clear()
            self.emit.schedule_render(after_ms=16)
            return {"content": [{"type": "text", "text": f"Cleared {count} note(s)."}]}
        else:
            return {"content": [{"type": "text", "text": f"Unknown tool: {tool_name}"}]}


if __name__ == "__main__":
    McpDemoApp().run()

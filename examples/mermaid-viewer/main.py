#!/usr/bin/env python3
"""
mermaid-viewer — v2.1 reference app for Plexi viewport, modal, and tabs primitives.

Displays a Mermaid diagram file with pan+zoom viewport interaction.
Demonstrates: ctx.viewport, ctx.modal, ctx.tabs, keyboard-driven zoom/pan.
"""
from __future__ import annotations

import os
import sys

# Allow running directly from examples/ dir by adding parent to path.
_sdk_path = os.path.join(os.path.dirname(__file__), "..", "..", "sdk", "python")
if os.path.isdir(_sdk_path):
    sys.path.insert(0, _sdk_path)

from plexi_sdk import App, RenderContext

app = App()

# ── State ──────────────────────────────────────────────────────────────────────
state = {
    "zoom": 1.0,
    "pan_x": 0.0,
    "pan_y": 0.0,
    "file_path": None,
    "file_content": "",
    "active_tab": "diagram",   # "diagram" | "source"
    "modal_visible": False,
    "modal_message": "",
}

ZOOM_STEP = 0.15
MIN_ZOOM = 0.2
MAX_ZOOM = 5.0
PAN_STEP = 30.0

# ── Helpers ────────────────────────────────────────────────────────────────────

def load_file(path: str):
    try:
        with open(path, "r") as f:
            state["file_content"] = f.read()
        state["file_path"] = path
    except OSError as e:
        state["modal_message"] = f"Could not open file:\n{e}"
        state["modal_visible"] = True


def render_diagram_nodes(ctx: RenderContext):
    """Render a simplified visual representation of the diagram content."""
    content = state["file_content"]
    lines = [l.strip() for l in content.splitlines() if l.strip()]
    y = 20.0
    for line in lines:
        color = "#89b4fa" if "-->" in line else "#cdd6f4"
        ctx.text(20, y, line, size=13.0, color=color, monospace=True)
        y += 20.0


def render_source_tab(ctx: RenderContext):
    """Render raw source text."""
    content = state["file_content"] or "(no file loaded)"
    lines = content.splitlines()
    y = 10.0
    for line in lines:
        ctx.text(10, y, line, size=12.0, color="#a6e3a1", monospace=True)
        y += 16.0


# ── Event handlers ─────────────────────────────────────────────────────────────

@app.on_init
def on_init(data, emit):
    intent = data.get("open_intent")
    if intent and intent.get("kind") == "file":
        load_file(intent["path"])
    elif len(sys.argv) > 1:
        load_file(sys.argv[1])
    else:
        state["file_content"] = "graph TD\n    A[Start] --> B[Process]\n    B --> C{Decision}\n    C -->|Yes| D[Done]\n    C -->|No| B"


@app.on_render
def on_render(ctx: RenderContext):
    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")

    # Tab bar
    tabs_result = ctx.tabs(
        "main-tabs",
        [("diagram", "Diagram"), ("source", "Source")],
        selected=state["active_tab"],
        height=32,
        x=0, y=0, w=ctx.width,
    )
    content_y = tabs_result["h"]
    content_h = ctx.height - content_y

    if state["active_tab"] == "diagram":
        # Zoom indicator
        zoom_text = f"zoom: {state['zoom']:.2f}x  pan: ({state['pan_x']:.0f}, {state['pan_y']:.0f})"
        ctx.text(ctx.width - 200, content_y + 6, zoom_text, size=11.0, color="#6c7086")

        # Help hint
        ctx.text(8, content_y + 6, "+/- zoom  arrow keys pan  r reset  ? help", size=11.0, color="#6c7086")

        # Viewport with zoom+pan
        ctx.viewport(
            "diagram-viewport",
            render_diagram_nodes,
            zoom=state["zoom"],
            pan=(state["pan_x"], state["pan_y"]),
            x=0, y=content_y + 24,
            w=ctx.width, h=content_h - 24,
            min_zoom=MIN_ZOOM, max_zoom=MAX_ZOOM,
        )
    else:
        render_source_tab(ctx)

    # Modal overlay (help or error)
    def modal_content(ctx, mx, my, mw, mh):
        ctx.text(mx + 16, my + 16, state["modal_message"], size=13.0, color="#cdd6f4")
        ctx.text(mx + 16, my + mh - 28, "Press Esc or Enter to close", size=11.0, color="#6c7086")

    ctx.modal(
        "info-modal",
        visible=state["modal_visible"],
        content_fn=modal_content,
        width=420, height=180,
        backdrop_alpha=160,
    )


@app.on_key
def on_key(key, mods, emit):
    if state["modal_visible"]:
        if key in ("Escape", "Enter"):
            state["modal_visible"] = False
        return

    if key == "=" or key == "+":
        state["zoom"] = min(MAX_ZOOM, state["zoom"] + ZOOM_STEP)
    elif key == "-":
        state["zoom"] = max(MIN_ZOOM, state["zoom"] - ZOOM_STEP)
    elif key == "r":
        state["zoom"] = 1.0
        state["pan_x"] = 0.0
        state["pan_y"] = 0.0
    elif key == "ArrowLeft":
        state["pan_x"] -= PAN_STEP
    elif key == "ArrowRight":
        state["pan_x"] += PAN_STEP
    elif key == "ArrowUp":
        state["pan_y"] -= PAN_STEP
    elif key == "ArrowDown":
        state["pan_y"] += PAN_STEP
    elif key == "?" or (key == "/" and mods.get("shift")):
        state["modal_message"] = "+/-  zoom in/out\narrow keys  pan\nr  reset zoom + pan\n1  fit to window\n? or Shift+/  this help"
        state["modal_visible"] = True
    elif key == "1":
        state["zoom"] = 1.0
        state["pan_x"] = 0.0
        state["pan_y"] = 0.0
    elif key == "Tab":
        state["active_tab"] = "source" if state["active_tab"] == "diagram" else "diagram"


@app.on_command
def on_command(text, emit):
    parts = text.strip().split(None, 1)
    cmd = parts[0] if parts else ""
    arg = parts[1] if len(parts) > 1 else ""
    if cmd == "open" and arg:
        load_file(arg)
    elif cmd == "zoom" and arg:
        try:
            state["zoom"] = max(MIN_ZOOM, min(MAX_ZOOM, float(arg)))
        except ValueError:
            pass
    elif cmd == "reset":
        state["zoom"] = 1.0
        state["pan_x"] = 0.0
        state["pan_y"] = 0.0


app.run()

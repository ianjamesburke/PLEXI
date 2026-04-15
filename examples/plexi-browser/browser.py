#!/usr/bin/env python3
"""
browser.py — Plexi manifest browser app

Navigate to a URL and discover whether the site has a Plexi manifest
at /.well-known/plexi.json. If found, renders the manifest's draw commands.

Controls:
  Type         Build URL input
  Backspace    Remove last character
  Escape       Clear URL input
  Enter        Fetch manifest from URL
"""

import json
import queue
import sys
import os
import threading
import urllib.request
import urllib.error
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "header":  "#181825",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "red":     "#f38ba8",
}

HEADER_H = 52
PADDING  = 16

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

url_input   = ""
status      = "Enter a URL and press Enter"
manifest    = None          # dict if found, None otherwise
fetch_state = "idle"        # "idle" | "fetching" | "found" | "not_found" | "error"
last_url    = ""            # the URL that was last fetched

result_queue: queue.Queue = queue.Queue()

# ---------------------------------------------------------------------------
# URL normalisation
# ---------------------------------------------------------------------------

def normalize_url(raw: str) -> str:
    """
    Produce a base URL (no path) from raw user input.

    - localhost:PORT   → http://localhost:PORT
    - http(s)://...    → as-is (strip trailing slash)
    - bare domain      → https://...
    """
    raw = raw.strip().rstrip("/")
    if not raw:
        return ""
    if raw.startswith("localhost") or raw.startswith("127.0.0.1"):
        return f"http://{raw}"
    if raw.startswith("http://") or raw.startswith("https://"):
        return raw
    return f"https://{raw}"


def manifest_url(base: str) -> str:
    return f"{base}/.well-known/plexi.json"

# ---------------------------------------------------------------------------
# Background fetch
# ---------------------------------------------------------------------------

def fetch_manifest(raw_input: str):
    base = normalize_url(raw_input)
    if not base:
        result_queue.put({"action": "error", "message": "Empty URL"})
        return

    url = manifest_url(base)
    try:
        req = urllib.request.Request(
            url,
            headers={"X-Plexi-Client": "1", "User-Agent": "Plexi/0.1"},
        )
        with urllib.request.urlopen(req, timeout=8) as resp:
            body = resp.read().decode("utf-8")
            data = json.loads(body)
            result_queue.put({"action": "found", "manifest": data})
    except urllib.error.HTTPError as e:
        if e.code == 404:
            result_queue.put({"action": "not_found"})
        else:
            result_queue.put({"action": "error", "message": f"HTTP {e.code}: {e.reason}"})
    except urllib.error.URLError as e:
        result_queue.put({"action": "error", "message": f"Connection failed: {e.reason}"})
    except json.JSONDecodeError as e:
        result_queue.put({"action": "error", "message": f"Invalid JSON in manifest: {e}"})
    except Exception as e:
        result_queue.put({"action": "error", "message": str(e)})


def start_fetch(raw_input: str):
    t = threading.Thread(target=fetch_manifest, args=(raw_input,), daemon=True)
    t.start()

# ---------------------------------------------------------------------------
# Manifest draw command rendering
# ---------------------------------------------------------------------------

def render_manifest_draw(ctx, commands: list, offset_y: float):
    """
    Replay draw commands from the manifest. Translate y by offset_y so
    commands land below the browser chrome (URL bar + info strip).
    """
    for cmd in commands:
        t = cmd.get("type")
        try:
            if t == "rect":
                ctx.rect(
                    cmd.get("x", 0),
                    cmd.get("y", 0) + offset_y,
                    cmd.get("w", 0),
                    cmd.get("h", 0),
                    fill=cmd.get("fill", "#000000"),
                    radius=cmd.get("radius", 0),
                )
            elif t == "text":
                ctx.text(
                    cmd.get("x", 0),
                    cmd.get("y", 0) + offset_y,
                    cmd.get("text", ""),
                    size=cmd.get("size", 13),
                    color=cmd.get("color", C["text"]),
                    bold=cmd.get("bold", False),
                    monospace=cmd.get("monospace", False),
                )
            elif t == "line":
                ctx.line(
                    cmd.get("x1", 0),
                    cmd.get("y1", 0) + offset_y,
                    cmd.get("x2", 0),
                    cmd.get("y2", 0) + offset_y,
                    color=cmd.get("color", C["text"]),
                    width=cmd.get("width", 1.0),
                )
        except Exception:
            pass  # skip malformed commands silently

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App()


@app.on_render
def render(ctx):
    global fetch_state, manifest, status, last_url

    # Drain result queue
    try:
        while True:
            msg = result_queue.get_nowait()
            action = msg["action"]
            if action == "found":
                manifest = msg["manifest"]
                fetch_state = "found"
                status = f"Manifest found: {manifest.get('name', 'Unknown')}"
            elif action == "not_found":
                manifest = None
                fetch_state = "not_found"
                status = "No Plexi manifest found"
            elif action == "error":
                manifest = None
                fetch_state = "error"
                status = msg.get("message", "Unknown error")
    except queue.Empty:
        pass

    w = ctx.width
    h = ctx.height

    # Background
    ctx.rect(0, 0, w, h, fill=C["bg"])

    # --- URL bar ---
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])

    url_display = f"URL: {url_input}_"
    ctx.text(PADDING, 10, url_display, size=13, color=C["text"])

    hint = "Enter=fetch  Esc=clear"
    hint_x = w - len(hint) * 7.2 - PADDING
    if hint_x > 0:
        ctx.text(hint_x, 10, hint, size=11, color=C["subtext"])

    # Status line
    status_color = C["subtext"]
    if fetch_state == "fetching":
        status_color = C["accent"]
    elif fetch_state == "error":
        status_color = C["red"]
    elif fetch_state == "found":
        status_color = C["green"]

    ctx.text(PADDING, 32, status, size=11, color=status_color)

    # Divider
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    # --- Body ---
    body_y = HEADER_H + PADDING

    if fetch_state == "idle":
        ctx.text(PADDING, body_y, "Enter a URL above and press Enter to check for a Plexi manifest.",
                 size=13, color=C["subtext"])

    elif fetch_state == "fetching":
        base = normalize_url(last_url)
        msg_text = f"Fetching /.well-known/plexi.json from {base}\u2026"
        text_x = w / 2 - len(msg_text) * 3.5
        ctx.text(max(PADDING, text_x), h / 2 - 8, msg_text, size=14, color=C["accent"])

    elif fetch_state == "found" and manifest is not None:
        # Show manifest info strip
        name = manifest.get("name", "Unknown")
        desc = manifest.get("description", "")
        version = manifest.get("version", "")

        info_strip_h = 0
        ctx.text(PADDING, body_y, name, size=16, color=C["text"], bold=True)
        info_strip_h += 24
        if desc:
            ctx.text(PADDING, body_y + info_strip_h, desc, size=12, color=C["subtext"])
            info_strip_h += 18
        if version:
            ctx.text(PADDING, body_y + info_strip_h, f"v{version}", size=11, color=C["subtext"])
            info_strip_h += 16

        divider_y = body_y + info_strip_h + 4
        ctx.line(0, divider_y, w, divider_y, color=C["surface"], width=1.0)

        content_offset_y = divider_y + 8

        draw_cmds = manifest.get("draw", [])
        if draw_cmds:
            render_manifest_draw(ctx, draw_cmds, offset_y=content_offset_y)
        else:
            ctx.text(PADDING, content_offset_y, "Manifest has no draw commands.",
                     size=13, color=C["subtext"])

    elif fetch_state == "not_found":
        ctx.text(PADDING, body_y,
                 f"No Plexi manifest found at {last_url}",
                 size=14, color=C["text"])
        ctx.text(PADDING, body_y + 24,
                 "This site hasn't opted into Plexi native rendering.",
                 size=12, color=C["subtext"])

    elif fetch_state == "error":
        ctx.text(PADDING, body_y, "Error:", size=14, color=C["red"], bold=True)
        ctx.text(PADDING, body_y + 22, status, size=12, color=C["red"])


@app.on_key
def on_key(key, _mods, _emit):
    global url_input, fetch_state, status, last_url

    if fetch_state == "fetching":
        return  # ignore input while loading

    if key == "Escape":
        url_input = ""
        fetch_state = "idle"
        status = "Enter a URL and press Enter"
        manifest = None

    elif key == "Backspace":
        url_input = url_input[:-1]

    elif key == "Enter":
        if url_input.strip():
            last_url = url_input.strip()
            fetch_state = "fetching"
            status = f"Fetching from {normalize_url(last_url)}\u2026"
            start_fetch(last_url)

    elif len(key) == 1 and key.isprintable():
        url_input += key


app.run()

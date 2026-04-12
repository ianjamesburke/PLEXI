#!/usr/bin/env python3
"""
weather — Plexi app
Current weather from wttr.in. Auto-refreshes every 5 minutes.

Controls:
  r    Force refresh
"""
from __future__ import annotations

import json
import math
import os
import queue
import sys
import threading
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "overlay": "#45475a",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "yellow":  "#f9e2af",
    "red":     "#f38ba8",
    "header":  "#181825",
    "mauve":   "#cba6f7",
}

PADDING   = 20
HEADER_H  = 48
AUTO_REFRESH_S = 300  # 5 minutes

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

weather_data: dict | None = None
error_msg: str = ""
loading: bool = False
last_fetch_ts: float = 0.0

result_q: queue.Queue = queue.Queue()

# ---------------------------------------------------------------------------
# Condition icons (wttr.in weatherCode ranges)
# ---------------------------------------------------------------------------

def condition_icon(code: int) -> str:
    if code == 113:
        return "☀️"
    if code in (116, 119, 122):
        return "☁️"
    if code in (143, 248, 260):
        return "🌫️"
    if code in (176, 179, 182, 185, 281, 284, 293, 296, 299, 302, 305, 308,
                311, 314, 317, 320, 323, 326):
        return "🌧️"
    if code in (200, 386, 389, 392, 395):
        return "⛈️"
    if code in (227, 230, 338, 335, 332, 329, 326, 323):
        return "❄️"
    if code in (263, 266, 353, 356, 359, 362, 365, 368, 371, 374, 377):
        return "🌦️"
    return "🌡️"

# ---------------------------------------------------------------------------
# Fetch
# ---------------------------------------------------------------------------

def fetch_weather():
    try:
        url = "https://wttr.in/?format=j1"
        req = urllib.request.Request(
            url,
            headers={"User-Agent": "Plexi/0.1 (https://github.com/ianjamesburke/PLEXI)"},
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())
        result_q.put({"ok": True, "data": data})
    except Exception as exc:
        result_q.put({"ok": False, "error": str(exc)})


def start_fetch():
    global loading
    loading = True
    t = threading.Thread(target=fetch_weather, daemon=True)
    t.start()

# ---------------------------------------------------------------------------
# Parse helpers
# ---------------------------------------------------------------------------

def parse_data(raw: dict) -> dict:
    try:
        nearest = raw["nearest_area"][0]
        area = nearest.get("areaName", [{}])[0].get("value", "Unknown")
        country = nearest.get("country", [{}])[0].get("value", "")
        location = f"{area}, {country}" if country else area

        current = raw["current_condition"][0]
        temp_c = int(current.get("temp_C", 0))
        temp_f = int(current.get("temp_F", 0))
        feels_c = int(current.get("FeelsLikeC", 0))
        feels_f = int(current.get("FeelsLikeF", 0))
        humidity = int(current.get("humidity", 0))
        wind_kmph = int(current.get("windspeedKmph", 0))
        desc = current.get("weatherDesc", [{}])[0].get("value", "")
        code = int(current.get("weatherCode", 113))

        return {
            "location": location,
            "temp_c": temp_c,
            "temp_f": temp_f,
            "feels_c": feels_c,
            "feels_f": feels_f,
            "humidity": humidity,
            "wind_kmph": wind_kmph,
            "desc": desc,
            "code": code,
            "icon": condition_icon(code),
        }
    except Exception as exc:
        return {"error": f"Parse error: {exc}"}

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="weather")


@app.on_render
def render(ctx):
    global weather_data, error_msg, loading, last_fetch_ts

    now = time.monotonic()

    # Drain queue
    try:
        while True:
            msg = result_q.get_nowait()
            loading = False
            if msg["ok"]:
                parsed = parse_data(msg["data"])
                if "error" in parsed:
                    error_msg = parsed["error"]
                    weather_data = None
                else:
                    weather_data = parsed
                    error_msg = ""
                    last_fetch_ts = now
            else:
                error_msg = msg["error"]
    except queue.Empty:
        pass

    # Auto-refresh
    if not loading and (now - last_fetch_ts) >= AUTO_REFRESH_S:
        start_fetch()

    w = ctx.width
    h = ctx.height

    # Background
    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 14, "Weather", size=14, color=C["accent"], bold=True)
    hint = "r=refresh"
    ctx.text(w - len(hint) * 7.5 - PADDING, 16, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    y = HEADER_H + PADDING

    if loading and weather_data is None:
        ctx.text(PADDING, y, "Fetching weather…", size=13, color=C["subtext"])
        return

    if error_msg and weather_data is None:
        ctx.text(PADDING, y, f"Error: {error_msg}", size=13, color=C["red"])
        ctx.text(PADDING, y + 22, "Press r to retry.", size=11, color=C["subtext"])
        return

    if weather_data is None:
        return

    d = weather_data

    # Big icon
    ctx.text(PADDING, y, d["icon"], size=52, color=C["text"])

    # Location
    ctx.text(PADDING + 80, y + 4, d["location"], size=16, color=C["text"], bold=True)

    # Condition description
    ctx.text(PADDING + 80, y + 28, d["desc"], size=13, color=C["subtext"])

    y += 70

    # Temperature row
    ctx.rect(PADDING, y, w - PADDING * 2, 56, fill=C["surface"], radius=8.0)
    ctx.text(PADDING + 16, y + 10, f"{d['temp_c']}°C", size=22, color=C["yellow"], bold=True)
    ctx.text(PADDING + 16, y + 36, "Temperature", size=10, color=C["subtext"])
    sep_x = PADDING + 16 + 90
    ctx.text(sep_x, y + 10, f"{d['temp_f']}°F", size=22, color=C["mauve"], bold=True)
    ctx.text(sep_x, y + 36, "Fahrenheit", size=10, color=C["subtext"])

    y += 68

    # Details row — feels like / humidity / wind
    col_w = (w - PADDING * 2) / 3
    labels = [
        (f"{d['feels_c']}°C / {d['feels_f']}°F", "Feels like"),
        (f"{d['humidity']}%",                      "Humidity"),
        (f"{d['wind_kmph']} km/h",                 "Wind"),
    ]
    for i, (val, lbl) in enumerate(labels):
        cx = PADDING + i * col_w
        ctx.rect(cx, y, col_w - 4, 52, fill=C["surface"], radius=8.0)
        ctx.text(cx + 10, y + 10, val, size=15, color=C["text"], bold=True)
        ctx.text(cx + 10, y + 32, lbl, size=10, color=C["subtext"])

    # Last updated
    if last_fetch_ts:
        age_s = int(now - last_fetch_ts)
        if age_s < 60:
            age_str = "just now"
        else:
            age_str = f"{age_s // 60}m ago"
        ctx.text(PADDING, h - PADDING, f"Updated {age_str}", size=10, color=C["subtext"])


@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global loading
    if key == "r" and not loading:
        start_fetch()


# Kick off initial fetch
start_fetch()

app.run()

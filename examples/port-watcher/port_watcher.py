#!/usr/bin/env python3
from __future__ import annotations
"""
port_watcher — Plexi app
Live visual network port monitor. Listening ports appear as hive cells;
active connections orbit them as animated bee dots.
"""

import math
import os
import subprocess
import sys
import time
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------

BG        = "#1e1e2e"
SURFACE   = "#313244"
TEXT      = "#cdd6f4"
SUBTEXT   = "#6c7086"
BLUE      = "#89b4fa"   # web servers
GREEN     = "#a6e3a1"   # databases
ORANGE    = "#fab387"   # unknown
BEE       = "#f9e2af"
BEE_FLASH = "#ffffff"
BORDER_DIM= "#45475a"

WEB_PROCS = {"python", "python3", "node", "nginx", "apache", "ruby", "uvicorn",
             "gunicorn", "caddy", "httpd", "deno", "bun"}
DB_PROCS  = {"postgres", "postgresql", "mysqld", "mongod", "redis-server",
             "mariadb", "sqlite", "pgbouncer", "mysql"}

def proc_color(name: str) -> str:
    n = name.lower()
    if any(p in n for p in WEB_PROCS):
        return BLUE
    if any(p in n for p in DB_PROCS):
        return GREEN
    return ORANGE

# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------

class Connection:
    def __init__(self, remote: str, state: str):
        self.remote  = remote
        self.state   = state
        self.born    = time.monotonic()
        self.angle   = 0.0        # orbit angle, radians
        self.flash   = 3          # frames remaining for new-bee flash

class PortEntry:
    def __init__(self, port: int, pid: int, proc: str):
        self.port  = port
        self.pid   = pid
        self.proc  = proc
        self.conns: list[Connection] = []
        self.pulse_t = 0.0        # time of last high-water-mark

# ---------------------------------------------------------------------------
# lsof polling
# ---------------------------------------------------------------------------

def _poll_lsof() -> dict[int, PortEntry]:
    """Run lsof and return a dict keyed by port number."""
    try:
        result = subprocess.run(
            ["lsof", "-iTCP", "-n", "-P", "-sTCP:LISTEN,ESTABLISHED"],
            capture_output=True, text=True, timeout=5,
        )
        lines = result.stdout.splitlines()
    except Exception:
        return {}

    entries: dict[int, PortEntry] = {}
    listen_ports: set[int] = set()

    # First pass — collect LISTEN rows to know which ports are our hive cells
    for line in lines[1:]:
        parts = line.split()
        if len(parts) < 10:
            continue
        proc   = parts[0]
        pid_s  = parts[1]
        name_f = parts[8] if len(parts) > 8 else ""
        state  = parts[9] if len(parts) > 9 else ""
        if state != "(LISTEN)":
            continue
        try:
            pid  = int(pid_s)
            port = int(name_f.rsplit(":", 1)[-1])
        except (ValueError, IndexError):
            continue
        if port not in entries:
            entries[port] = PortEntry(port, pid, proc)
        listen_ports.add(port)

    # Second pass — attach ESTABLISHED connections to matching local port
    for line in lines[1:]:
        parts = line.split()
        if len(parts) < 10:
            continue
        name_f = parts[8] if len(parts) > 8 else ""
        state  = parts[9] if len(parts) > 9 else ""
        if state != "(ESTABLISHED)":
            continue
        # name_f looks like  local:port->remote:port
        try:
            arrow_idx = name_f.index("->")
            local_part  = name_f[:arrow_idx]
            remote_part = name_f[arrow_idx + 2:]
            local_port  = int(local_part.rsplit(":", 1)[-1])
        except (ValueError, IndexError):
            continue
        if local_port in entries:
            entries[local_port].conns.append(Connection(remote_part, "ESTABLISHED"))

    return entries

# ---------------------------------------------------------------------------
# App state
# ---------------------------------------------------------------------------

_ports: dict[int, PortEntry] = {}
_port_keys: list[int] = []      # sorted list of ports (grid order)
_selected_idx: int = 0
_detail_mode: bool = False
_detail_scroll: int = 0
_filter: str = ""
_filter_mode: bool = False
_kill_confirm: bool = False
_last_poll: float = 0.0
_start: float = time.monotonic()
_frame: int = 0

POLL_INTERVAL = 1.0

def _now() -> float:
    return time.monotonic() - _start

def _refresh():
    global _ports, _port_keys, _selected_idx
    raw = _poll_lsof()

    # Merge: preserve existing Connection objects (for orbit continuity), add new ones
    next_ports: dict[int, PortEntry] = {}
    for port, entry in raw.items():
        if port in _ports:
            existing = _ports[port]
            # Update metadata
            existing.proc = entry.proc
            existing.pid  = entry.pid
            # Reconcile connections: keep existing where remote matches
            old_remotes = {c.remote: c for c in existing.conns}
            new_conns = []
            for c in entry.conns:
                if c.remote in old_remotes:
                    new_conns.append(old_remotes[c.remote])
                else:
                    new_conns.append(c)
            existing.conns = new_conns
            if len(existing.conns) > 10:
                existing.pulse_t = time.monotonic()
            next_ports[port] = existing
        else:
            if len(entry.conns) > 10:
                entry.pulse_t = time.monotonic()
            next_ports[port] = entry

    _ports = next_ports
    _filtered_keys = _visible_keys()
    # Clamp selection
    if _selected_idx >= len(_filtered_keys):
        _selected_idx = max(0, len(_filtered_keys) - 1)
    _port_keys = _filtered_keys

def _visible_keys() -> list[int]:
    keys = sorted(_ports.keys())
    if _filter:
        fl = _filter.lower()
        keys = [p for p in keys if fl in str(p) or fl in _ports[p].proc.lower()]
    return keys

# ---------------------------------------------------------------------------
# Grid layout helpers
# ---------------------------------------------------------------------------

CELL_W = 160.0
CELL_H = 100.0
CELL_PAD = 14.0
HEADER_H = 36.0

def _grid_cols(width: float) -> int:
    return max(1, int(width // (CELL_W + CELL_PAD)))

def _cell_rect(idx: int, cols: int) -> tuple[float, float, float, float]:
    row = idx // cols
    col = idx % cols
    x = CELL_PAD + col * (CELL_W + CELL_PAD)
    y = HEADER_H + CELL_PAD + row * (CELL_H + CELL_PAD)
    return x, y, CELL_W, CELL_H

# ---------------------------------------------------------------------------
# Drawing
# ---------------------------------------------------------------------------

def _draw_header(ctx, width: float):
    ctx.rect(0, 0, width, HEADER_H, fill=SURFACE)
    title = "Port Watcher"
    if _filter_mode:
        title = f"Filter: {_filter}_"
    elif _filter:
        title = f"Port Watcher  [filter: {_filter}]"
    ctx.text(12, 10, title, size=14, color=TEXT, bold=True)
    hints = "j/k nav  Enter detail  f filter  r refresh  q quit"
    ctx.text(width - 420, 11, hints, size=11, color=SUBTEXT)

def _draw_cell(ctx, entry: PortEntry, x: float, y: float, w: float, h: float,
               selected: bool, now: float):
    # Pulse effect for high-traffic ports
    pulse = 0.0
    age = now - (entry.pulse_t - _start)
    if age < 1.5:
        pulse = math.sin(age * math.pi * 4) * (1.0 - age / 1.5)
    pulse = max(0.0, pulse)

    border_col = proc_color(entry.proc) if selected else BORDER_DIM
    bg_col     = SURFACE

    # Outer border rect
    bw = 2.0 + pulse * 2.0
    ctx.rect(x - bw, y - bw, w + bw * 2, h + bw * 2, fill=border_col, radius=8.0)
    ctx.rect(x, y, w, h, fill=bg_col, radius=6.0)

    # Port number
    ctx.text(x + 10, y + 10, str(entry.port), size=22, color=proc_color(entry.proc), bold=True)

    # Process name (truncated)
    pname = entry.proc[:16]
    ctx.text(x + 10, y + 40, pname, size=11, color=SUBTEXT)

    # Connection count
    nc = len(entry.conns)
    conn_label = f"{nc} conn" if nc != 1 else "1 conn"
    ctx.text(x + 10, y + 58, conn_label, size=11, color=TEXT)

    # PID
    ctx.text(x + 10, y + 74, f"pid {entry.pid}", size=10, color=SUBTEXT)

    # Bee dots orbiting the cell
    _draw_bees(ctx, entry, x + w / 2, y + h / 2, w / 2 + 18, now)

def _draw_bees(ctx, entry: PortEntry, cx: float, cy: float,
               orbit_r: float, now: float):
    nc = len(entry.conns)
    if nc == 0:
        return
    # Orbit speed: faster with more connections, capped
    speed = min(0.4 + nc * 0.05, 1.2)
    for i, conn in enumerate(entry.conns[:20]):  # cap rendered bees at 20
        base_angle = (2 * math.pi * i / max(nc, 1))
        conn.angle = base_angle + now * speed
        bx = cx + orbit_r * math.cos(conn.angle) - 2
        by = cy + orbit_r * math.sin(conn.angle) - 2
        color = BEE_FLASH if conn.flash > 0 else BEE
        if conn.flash > 0:
            conn.flash -= 1
        ctx.rect(bx, by, 4, 4, fill=color, radius=2.0)

def _draw_grid(ctx, now: float):
    w = ctx.width
    cols = _grid_cols(w)
    for i, port in enumerate(_port_keys):
        entry = _ports.get(port)
        if entry is None:
            continue
        x, y, cw, ch = _cell_rect(i, cols)
        if y > ctx.height:
            break  # off screen
        _draw_cell(ctx, entry, x, y, cw, ch, selected=(i == _selected_idx), now=now)

def _draw_detail(ctx, now: float):
    port = _port_keys[_selected_idx] if _port_keys else None
    if port is None:
        return
    entry = _ports.get(port)
    if entry is None:
        return

    w, h = ctx.width, ctx.height
    ctx.rect(0, 0, w, h, fill=BG)

    # Header bar
    ctx.rect(0, 0, w, 40, fill=SURFACE)
    ctx.text(12, 10, f"Port {entry.port}  —  {entry.proc}  (pid {entry.pid})",
             size=14, color=proc_color(entry.proc), bold=True)
    ctx.text(w - 120, 13, "Esc to go back", size=11, color=SUBTEXT)

    # Column headers
    ctx.text(16, 52, "REMOTE", size=11, color=SUBTEXT, bold=True)
    ctx.text(300, 52, "STATE", size=11, color=SUBTEXT, bold=True)
    ctx.text(420, 52, "AGE", size=11, color=SUBTEXT, bold=True)
    ctx.line(16, 68, w - 16, 68, color=BORDER_DIM, width=1.0)

    row_h = 28.0
    start_y = 76.0
    conns = entry.conns

    # Kill confirm overlay
    if _kill_confirm:
        ctx.rect(w // 2 - 140, h // 2 - 40, 280, 80, fill=SURFACE, radius=8.0)
        ctx.text(w // 2 - 120, h // 2 - 24,
                 f"Kill process {entry.pid}? (y/n)", size=13, color="#f38ba8", bold=True)
        return

    for i, conn in enumerate(conns[_detail_scroll:_detail_scroll + 30]):
        ry = start_y + i * row_h
        if ry + row_h > h:
            break
        age_s = int(now - (conn.born - _start))
        age_label = f"{age_s}s" if age_s < 60 else f"{age_s // 60}m{age_s % 60}s"
        color = BEE_FLASH if conn.flash > 0 else TEXT
        ctx.text(16,  ry, conn.remote[:35], size=12, color=color)
        ctx.text(300, ry, conn.state,       size=12, color=SUBTEXT)
        ctx.text(420, ry, age_label,         size=12, color=SUBTEXT)

    if not conns:
        ctx.text(16, start_y, "No active connections.", size=12, color=SUBTEXT)

def _draw_empty(ctx):
    ctx.text(ctx.width / 2 - 80, ctx.height / 2 - 10,
             "No listening ports found.", size=13, color=SUBTEXT)
    ctx.text(ctx.width / 2 - 60, ctx.height / 2 + 12,
             "Press r to refresh.", size=11, color=SUBTEXT)

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="port-watcher")

@app.on_render
def render(ctx):
    global _last_poll, _frame
    _frame += 1
    now = _now()

    # Poll on interval
    if now - _last_poll >= POLL_INTERVAL or _last_poll == 0.0:
        _refresh()
        _last_poll = now

    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill=BG)

    if _detail_mode and _port_keys:
        _draw_detail(ctx, now)
    else:
        _draw_header(ctx, ctx.width)
        if _port_keys:
            _draw_grid(ctx, now)
        else:
            _draw_empty(ctx)

@app.on_key
def on_key(key: str, mods: dict, emit):
    global _selected_idx, _detail_mode, _detail_scroll
    global _filter, _filter_mode, _kill_confirm, _last_poll

    if _filter_mode:
        if key == "Escape" or key == "Enter":
            _filter_mode = False
            _refresh()
        elif key == "Backspace":
            _filter = _filter[:-1]
        elif len(key) == 1:
            _filter += key
        return

    if _kill_confirm:
        if key == "y":
            port = _port_keys[_selected_idx] if _port_keys else None
            if port is not None:
                entry = _ports.get(port)
                if entry:
                    try:
                        subprocess.run(["kill", str(entry.pid)], timeout=3)
                    except Exception:
                        pass
                    _last_poll = 0.0  # force refresh
        _kill_confirm = False
        return

    if key == "Escape":
        _detail_mode = False
        _kill_confirm = False

    elif key in ("j", "ArrowDown"):
        if _detail_mode:
            _detail_scroll += 1
        else:
            _selected_idx = min(_selected_idx + 1, max(0, len(_port_keys) - 1))

    elif key in ("k", "ArrowUp"):
        if _detail_mode:
            _detail_scroll = max(0, _detail_scroll - 1)
        else:
            _selected_idx = max(0, _selected_idx - 1)

    elif key in ("h", "ArrowLeft") and not _detail_mode:
        _selected_idx = max(0, _selected_idx - 1)

    elif key in ("l", "ArrowRight") and not _detail_mode:
        _selected_idx = min(_selected_idx + 1, max(0, len(_port_keys) - 1))

    elif key == "Enter" and not _detail_mode and _port_keys:
        _detail_mode = True
        _detail_scroll = 0

    elif key == "f" and not _detail_mode:
        _filter_mode = True

    elif key == "r":
        _last_poll = 0.0  # trigger immediate refresh next render

    elif key == "K" and _detail_mode and _port_keys:
        _kill_confirm = True

@app.on_resize
def on_resize(width: float, height: float):
    pass  # layout is computed dynamically from ctx dimensions each frame

app.run()

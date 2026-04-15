#!/usr/bin/env python3
from __future__ import annotations
"""
port_watcher — Plexi app

Live network port monitor. Lists listening ports via lsof and shows
per-port process + live ESTABLISHED connection counts. Drill into a port
to see each connection's remote, state, and age.

Phase 1 SDK components: THEME, ctx.header, ctx.status_bar,
ctx.scrollable_list, ctx.scrollable_text, ctx.empty_state, ctx.text_right,
named sizes (BODY, CAPTION, MONO_BODY, HINT), PAD.
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import (
    App,
    THEME,
    BODY, CAPTION, HINT, MONO_BODY,
    PAD,
)

# ── process classification ────────────────────────────────────────────────────
WEB_PROCS = {"python", "python3", "node", "nginx", "apache", "ruby", "uvicorn",
             "gunicorn", "caddy", "httpd", "deno", "bun"}
DB_PROCS  = {"postgres", "postgresql", "mysqld", "mongod", "redis-server",
             "mariadb", "sqlite", "pgbouncer", "mysql"}


def proc_color(name: str) -> str:
    n = name.lower()
    if any(p in n for p in WEB_PROCS):
        return THEME.accent  # blue
    if any(p in n for p in DB_PROCS):
        return THEME.green
    return THEME.yellow


# ── data model ────────────────────────────────────────────────────────────────
class Connection:
    def __init__(self, remote: str, state: str):
        self.remote = remote
        self.state  = state
        self.born   = time.monotonic()


class PortEntry:
    def __init__(self, port: int, pid: int, proc: str):
        self.port = port
        self.pid  = pid
        self.proc = proc
        self.conns: list[Connection] = []


# ── lsof polling ──────────────────────────────────────────────────────────────
def _poll_lsof() -> dict[int, PortEntry]:
    """Run lsof and return a dict keyed by listening port number."""
    try:
        result = subprocess.run(
            ["lsof", "-iTCP", "-n", "-P", "-sTCP:LISTEN,ESTABLISHED"],
            capture_output=True, text=True, timeout=5,
        )
        lines = result.stdout.splitlines()
    except Exception:
        return {}

    entries: dict[int, PortEntry] = {}

    # First pass — collect LISTEN rows
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

    # Second pass — attach ESTABLISHED connections to matching local port
    for line in lines[1:]:
        parts = line.split()
        if len(parts) < 10:
            continue
        name_f = parts[8] if len(parts) > 8 else ""
        state  = parts[9] if len(parts) > 9 else ""
        if state != "(ESTABLISHED)":
            continue
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


# ── state ─────────────────────────────────────────────────────────────────────
_ports: dict[int, PortEntry] = {}
_port_keys: list[int] = []       # sorted + filtered visible port list
_selected: int = 0
_detail_mode: bool = False
_detail_scroll: int = 0
_filter: str = ""
_filter_mode: bool = False
_kill_confirm: bool = False
_last_poll: float = 0.0
_start: float = time.monotonic()

POLL_INTERVAL = 1.0


def _now() -> float:
    return time.monotonic() - _start


def _visible_keys() -> list[int]:
    keys = sorted(_ports.keys())
    if _filter:
        fl = _filter.lower()
        keys = [p for p in keys if fl in str(p) or fl in _ports[p].proc.lower()]
    return keys


def _refresh() -> None:
    global _ports, _port_keys, _selected
    raw = _poll_lsof()

    # Merge: preserve existing Connection objects (for born timestamps)
    next_ports: dict[int, PortEntry] = {}
    for port, entry in raw.items():
        if port in _ports:
            existing = _ports[port]
            existing.proc = entry.proc
            existing.pid  = entry.pid
            old_remotes = {c.remote: c for c in existing.conns}
            new_conns = []
            for c in entry.conns:
                if c.remote in old_remotes:
                    new_conns.append(old_remotes[c.remote])
                else:
                    new_conns.append(c)
            existing.conns = new_conns
            next_ports[port] = existing
        else:
            next_ports[port] = entry

    _ports = next_ports
    _port_keys = _visible_keys()
    if _selected >= len(_port_keys):
        _selected = max(0, len(_port_keys) - 1)


def _clamp(idx: int) -> int:
    if not _port_keys:
        return 0
    return max(0, min(len(_port_keys) - 1, idx))


# ── rendering ─────────────────────────────────────────────────────────────────
app = App(app_id="port-watcher")


def _render_port_row(ctx, port, i, x, y, w, is_sel):
    row_h = 44.0
    bg = THEME.highlight if is_sel else THEME.bg
    ctx.rect(x + PAD / 2, y, w - PAD, row_h, fill=bg, radius=6)

    entry = _ports.get(port)
    if entry is None:
        return

    dot_color = proc_color(entry.proc)
    ctx.rect(x + PAD, y + (row_h - 8) / 2, 8, 8, fill=dot_color, radius=4)

    text_x = x + PAD + 22
    # Port number + process name, primary row
    port_label = str(entry.port)
    ctx.text(
        text_x, y + 6,
        port_label,
        size=BODY,
        color=dot_color,
        bold=True,
    )
    proc_x = text_x + ctx.measure_text(port_label, BODY, monospace=False) + 10
    ctx.text(
        proc_x, y + 6,
        entry.proc[:40],
        size=BODY,
        color=THEME.fg if is_sel else THEME.muted,
    )
    ctx.text(
        text_x, y + 6 + BODY + 2,
        f"pid {entry.pid}",
        size=CAPTION,
        color=THEME.muted,
    )

    # Right-aligned connection count.
    nc = len(entry.conns)
    conn_label = f"{nc} conn" if nc != 1 else "1 conn"
    right_edge = x + w - PAD
    ctx.text_right(
        right_edge, y + (row_h - BODY) / 2,
        conn_label,
        size=BODY,
        color=THEME.green if nc > 0 else THEME.muted,
        bold=nc > 0,
    )


def _render_detail(ctx, now: float) -> None:
    if not _port_keys:
        return
    port = _port_keys[_selected]
    entry = _ports.get(port)
    if entry is None:
        return

    ctx.header(
        f"Port {entry.port}  ·  {entry.proc}",
        subtitle=f"pid {entry.pid}  ·  {len(entry.conns)} connection{'s' if len(entry.conns) != 1 else ''}",
    )

    if _kill_confirm:
        ctx.empty_state(
            f"Kill process {entry.pid}?",
            subtitle="y to confirm  ·  n to cancel",
            icon_color=THEME.red,
        )
        ctx.status_bar(
            [("y", "confirm"), ("n", "cancel")],
        )
        return

    if not entry.conns:
        ctx.empty_state("No active connections", "Waiting for traffic…")
        ctx.status_bar(
            [("j/k", "scroll"), ("K", "kill proc"), ("Esc", "back"), ("⌘W", "close")],
        )
        return

    # Build lines for scrollable_text: "<remote>  <state>  <age>"
    lines: list[str] = []
    for conn in entry.conns:
        age_s = int(now - (conn.born - _start))
        age_label = f"{age_s}s" if age_s < 60 else f"{age_s // 60}m{age_s % 60}s"
        remote = conn.remote[:40].ljust(42)
        state = conn.state.ljust(14)
        lines.append(f"{remote}{state}{age_label}")

    global _detail_scroll
    _detail_scroll = ctx.scrollable_text(
        text_id="detail",
        lines=lines,
        scroll_offset=_detail_scroll,
        line_height=MONO_BODY + 4,
        size=MONO_BODY,
        monospace=True,
    )

    ctx.status_bar(
        [("j/k", "scroll"), ("K", "kill proc"), ("Esc", "back"), ("⌘W", "close")],
    )


@app.on_render
def render(ctx):
    global _last_poll
    now = _now()

    # Poll on interval.
    if now - _last_poll >= POLL_INTERVAL or _last_poll == 0.0:
        _refresh()
        _last_poll = now

    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    # ── detail view ──────────────────────────────────────────────────────────
    if _detail_mode and _port_keys:
        _render_detail(ctx, now)
        return

    # ── header ───────────────────────────────────────────────────────────────
    if _filter_mode:
        title = f"Filter: {_filter}_"
    elif _filter:
        title = f"Port Watcher  ·  {len(_port_keys)} match{'es' if len(_port_keys) != 1 else ''}  [{_filter}]"
    else:
        title = f"Port Watcher  ·  {len(_port_keys)} port{'s' if len(_port_keys) != 1 else ''}"
    ctx.header(title)

    # ── empty state ──────────────────────────────────────────────────────────
    if not _port_keys:
        if _filter:
            ctx.empty_state("No matches", f"No ports match '{_filter}'")
        else:
            ctx.empty_state("No listening ports", "Press r to refresh")
        ctx.status_bar(
            [("f", "filter"), ("r", "refresh"), ("⌘W", "close")],
        )
        return

    # ── list view ────────────────────────────────────────────────────────────
    ctx.scrollable_list(
        list_id="ports",
        items=_port_keys,
        selected=_selected,
        row_height=48.0,
        render_row=_render_port_row,
    )

    ctx.status_bar(
        [
            ("j/k", "navigate"),
            ("Enter", "detail"),
            ("f", "filter"),
            ("r", "refresh"),
            ("⌘W", "close"),
        ],
    )


# ── input ─────────────────────────────────────────────────────────────────────
@app.on_key
def on_key(key: str, mods: dict, emit):
    global _selected, _detail_mode, _detail_scroll
    global _filter, _filter_mode, _kill_confirm, _last_poll

    # Filter-entry mode.
    if _filter_mode:
        if key in ("Escape", "Enter"):
            _filter_mode = False
            _refresh()
        elif key == "Backspace":
            _filter = _filter[:-1]
        elif len(key) == 1:
            _filter += key
        return

    # Kill confirmation overlay (detail view).
    if _kill_confirm:
        if key == "y":
            port = _port_keys[_selected] if _port_keys else None
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
        if _detail_mode:
            _detail_mode = False
            _detail_scroll = 0
        _kill_confirm = False
        return

    # Detail view navigation.
    if _detail_mode:
        if key in ("j", "ArrowDown"):
            _detail_scroll += 1
        elif key in ("k", "ArrowUp"):
            _detail_scroll = max(0, _detail_scroll - 1)
        elif key == "K" and _port_keys:
            _kill_confirm = True
        return

    # List view navigation.
    if key in ("j", "ArrowDown"):
        _selected = _clamp(_selected + 1)
    elif key in ("k", "ArrowUp"):
        _selected = _clamp(_selected - 1)
    elif key == "Enter" and _port_keys:
        _detail_mode = True
        _detail_scroll = 0
    elif key == "f":
        _filter_mode = True
    elif key == "r":
        _last_poll = 0.0  # trigger immediate refresh next render


@app.on_resize
def on_resize(width: float, height: float):
    pass  # layout is computed dynamically from ctx dimensions each frame


app.run()

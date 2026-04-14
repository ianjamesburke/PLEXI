from __future__ import annotations
"""
permissions_viewer.py — Capability matrix for all installed Plexi apps.

Shows which apps have which capabilities (filesystem, terminal write,
mouse tracking, accepted file types). Color-coded: green = restricted,
yellow = moderate, red = broad access.

Keys:
  j / Down    next app
  k / Up      previous app
  r           reload from disk
  q / Esc     quit
"""

import os
import pathlib
from plexi_sdk import App, load_manifest

# ── palette ───────────────────────────────────────────────────────────────────
BG        = "#1e1e2e"
SURFACE   = "#313244"
HIGHLIGHT = "#45475a"
ACCENT    = "#89b4fa"
MUTED     = "#6c7086"
FG        = "#cdd6f4"
RED       = "#f38ba8"
GREEN     = "#a6e3a1"
YELLOW    = "#f9e2af"
BLUE      = "#89dceb"

# ── TOML micro-parser (reuses the same logic as load_manifest) ───────────────
def _parse_toml(text: str) -> dict:
    result: dict = {}
    section: dict = result
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            keys = line[1:-1].split(".")
            section = result
            for k in keys:
                section = section.setdefault(k, {})
            continue
        if "=" in line:
            k, _, v = line.partition("=")
            k, v = k.strip(), v.strip()
            if v.startswith('"') and v.endswith('"'):
                v = v[1:-1]
            elif v.startswith("[") and v.endswith("]"):
                inner = v[1:-1]
                v = [x.strip().strip('"') for x in inner.split(",") if x.strip()]  # type: ignore[assignment]
            elif v == "true":
                v = True  # type: ignore[assignment]
            elif v == "false":
                v = False  # type: ignore[assignment]
            else:
                try:
                    v = int(v)  # type: ignore[assignment]
                except ValueError:
                    pass
            section[k] = v
    return result


def _apps_dir() -> pathlib.Path:
    d = os.environ.get("PLEXI_APPS_DIR")
    if d:
        return pathlib.Path(d)
    return pathlib.Path.home() / ".plexi-alpha" / "apps"


def _load_apps() -> list[dict]:
    apps = []
    apps_dir = _apps_dir()
    if not apps_dir.exists():
        return apps
    for entry in sorted(apps_dir.iterdir()):
        if not entry.is_dir():
            continue
        manifest_path = entry / "manifest.toml"
        if not manifest_path.exists():
            continue
        try:
            raw = _parse_toml(manifest_path.read_text())
        except OSError:
            continue
        app_data = raw.get("app", {})
        caps = app_data.get("capabilities", {})
        apps.append({
            "id": app_data.get("id", entry.name),
            "name": app_data.get("name", entry.name),
            "version": app_data.get("version", "—"),
            "filesystem": caps.get("filesystem", "none"),
            "terminal_write": bool(caps.get("terminal_write", False)),
            "mouse_tracking": bool(caps.get("mouse_tracking", False)),
            "file_types": caps.get("file_types", []),
            "description": app_data.get("description", ""),
        })
    return apps


def _fs_color(fs: str) -> str:
    return {
        "none":  GREEN,
        "read":  YELLOW,
        "write": RED,
        "full":  RED,
    }.get(fs, MUTED)


def _bool_label(b: bool) -> str:
    return "yes" if b else "—"


def _bool_color(b: bool) -> str:
    return YELLOW if b else MUTED


# ── state ─────────────────────────────────────────────────────────────────────
apps: list[dict] = _load_apps()
selected: int = 0

manifest = load_manifest(__file__)
APP_VERSION = manifest.get("app", {}).get("version", "0.1.0")

app = App("permissions-viewer")


def _clamp(idx: int) -> int:
    if not apps:
        return 0
    return max(0, min(len(apps) - 1, idx))


@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=BG)

    # ── header ────────────────────────────────────────────────────────────────
    ctx.rect(0, 0, ctx.width, 36, fill=SURFACE)
    ctx.text(16, 10, "Permissions Viewer", size=14, color=ACCENT, bold=True)
    ctx.text(ctx.width - 170, 10, f"{len(apps)} apps  ·  v{APP_VERSION}", size=11, color=MUTED)

    if not apps:
        ctx.text(ctx.width / 2 - 100, ctx.height / 2, "No apps found in apps directory.", size=14, color=MUTED)
        return

    # ── column header row ─────────────────────────────────────────────────────
    col_y = 44.0
    ctx.rect(0, col_y, ctx.width, 24, fill=SURFACE)
    cols = _col_positions(ctx.width)
    ctx.text(cols["name"],       col_y + 5, "App",        size=11, color=MUTED, bold=True)
    ctx.text(cols["version"],    col_y + 5, "Version",    size=11, color=MUTED, bold=True)
    ctx.text(cols["filesystem"], col_y + 5, "Filesystem", size=11, color=MUTED, bold=True)
    ctx.text(cols["terminal"],   col_y + 5, "Terminal",   size=11, color=MUTED, bold=True)
    ctx.text(cols["mouse"],      col_y + 5, "Mouse",      size=11, color=MUTED, bold=True)
    ctx.text(cols["filetypes"],  col_y + 5, "File types", size=11, color=MUTED, bold=True)

    # ── rows ──────────────────────────────────────────────────────────────────
    row_h = 36.0
    list_top = col_y + 24
    visible = max(1, int((ctx.height - list_top - 56) / row_h))
    scroll_off = max(0, selected - visible + 1)

    for i in range(visible):
        ni = scroll_off + i
        if ni >= len(apps):
            break
        a = apps[ni]
        y = list_top + i * row_h
        is_sel = ni == selected

        bg = HIGHLIGHT if is_sel else (SURFACE if i % 2 == 0 else BG)
        ctx.rect(0, y, ctx.width, row_h - 1, fill=bg)

        name_color = ACCENT if is_sel else FG
        ctx.text(cols["name"],       y + 10, a["name"][:22],                             size=12, color=name_color, bold=is_sel)
        ctx.text(cols["version"],    y + 10, a["version"],                                size=11, color=MUTED)
        ctx.text(cols["filesystem"], y + 10, a["filesystem"],                             size=11, color=_fs_color(a["filesystem"]))
        ctx.text(cols["terminal"],   y + 10, _bool_label(a["terminal_write"]),            size=11, color=_bool_color(a["terminal_write"]))
        ctx.text(cols["mouse"],      y + 10, _bool_label(a["mouse_tracking"]),            size=11, color=_bool_color(a["mouse_tracking"]))
        fts = ", ".join(a["file_types"]) if a["file_types"] else "—"
        ctx.text(cols["filetypes"],  y + 10, fts[:24],                                   size=11, color=BLUE if a["file_types"] else MUTED)

    # ── detail panel for selected ─────────────────────────────────────────────
    if apps:
        a = apps[selected]
        detail_y = ctx.height - 52
        ctx.rect(0, detail_y, ctx.width, 52, fill=SURFACE)
        ctx.text(16, detail_y + 8, a["description"] or a["name"], size=12, color=FG)
        ctx.text(16, detail_y + 28, "j/k  navigate    r  reload    q  quit", size=10, color=MUTED)


def _col_positions(width: float) -> dict:
    return {
        "name":       16,
        "version":    200,
        "filesystem": 290,
        "terminal":   400,
        "mouse":      490,
        "filetypes":  570,
    }


@app.on_key
def on_key(key, mods, emit):
    global selected, apps

    if key in ("j", "ArrowDown"):
        selected = _clamp(selected + 1)
    elif key in ("k", "ArrowUp"):
        selected = _clamp(selected - 1)
    elif key == "r":
        apps = _load_apps()
        selected = _clamp(selected)
    elif key in ("q", "Escape"):
        emit.run_in_terminal("")


app.run()

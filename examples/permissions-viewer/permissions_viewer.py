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
  ⌘W          close
"""

import os
import pathlib

from plexi_sdk import (
    App, load_manifest,
    THEME,
    BODY, CAPTION, HINT,
    PAD, HEADER_H,
)


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
        "none":  THEME.green,
        "read":  THEME.yellow,
        "write": THEME.red,
        "full":  THEME.red,
    }.get(fs, THEME.muted)


def _bool_label(b: bool) -> str:
    return "yes" if b else "—"


def _bool_color(b: bool) -> str:
    return THEME.yellow if b else THEME.muted


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


# ── rendering ─────────────────────────────────────────────────────────────────
def _col_positions() -> dict:
    return {
        "name":       PAD,
        "version":    200.0,
        "filesystem": 290.0,
        "terminal":   400.0,
        "mouse":      490.0,
        "filetypes":  570.0,
    }


def _render_row(ctx, a, i, x, y, w, is_sel):
    row_h = 36.0
    bg = THEME.highlight if is_sel else (THEME.surface if i % 2 == 0 else THEME.bg)
    ctx.rect(x, y, w, row_h - 1, fill=bg)

    cols = _col_positions()
    name_color = THEME.accent if is_sel else THEME.fg
    text_y = y + (row_h - CAPTION) / 2

    ctx.text(cols["name"],       text_y, a["name"][:22],
             size=BODY, color=name_color, bold=is_sel)
    ctx.text(cols["version"],    text_y, a["version"],
             size=CAPTION, color=THEME.muted)
    ctx.text(cols["filesystem"], text_y, a["filesystem"],
             size=CAPTION, color=_fs_color(a["filesystem"]))
    ctx.text(cols["terminal"],   text_y, _bool_label(a["terminal_write"]),
             size=CAPTION, color=_bool_color(a["terminal_write"]))
    ctx.text(cols["mouse"],      text_y, _bool_label(a["mouse_tracking"]),
             size=CAPTION, color=_bool_color(a["mouse_tracking"]))
    fts = ", ".join(a["file_types"]) if a["file_types"] else "—"
    ctx.text(cols["filetypes"],  text_y, fts[:24],
             size=CAPTION,
             color=THEME.accent if a["file_types"] else THEME.muted)


@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    # ── header ────────────────────────────────────────────────────────────────
    ctx.header(
        "Permissions Viewer",
        subtitle=f"{len(apps)} apps  ·  v{APP_VERSION}",
    )

    # ── empty state ───────────────────────────────────────────────────────────
    if not apps:
        ctx.empty_state(
            "No apps found",
            subtitle="Nothing in the apps directory.",
        )
        ctx.status_bar([("⌘W", "close")])
        return

    # ── column header row ─────────────────────────────────────────────────────
    col_y = HEADER_H + PAD / 4
    ctx.rect(0, col_y, ctx.width, 24, fill=THEME.surface)
    cols = _col_positions()
    label_y = col_y + (24 - HINT) / 2
    for key, label in (
        ("name",       "App"),
        ("version",    "Version"),
        ("filesystem", "Filesystem"),
        ("terminal",   "Terminal"),
        ("mouse",      "Mouse"),
        ("filetypes",  "File types"),
    ):
        ctx.text(cols[key], label_y, label,
                 size=HINT, color=THEME.muted, bold=True)

    # ── rows ──────────────────────────────────────────────────────────────────
    row_h = 36.0
    list_top = col_y + 24
    # Reserve space for detail panel (52) + status bar (30) + padding.
    list_bottom = ctx.height - 52 - 30
    ctx.scrollable_list(
        list_id="permissions",
        items=apps,
        selected=selected,
        row_height=row_h,
        render_row=_render_row,
        x=0.0,
        y=list_top,
        w=ctx.width,
        h=max(row_h, list_bottom - list_top),
    )

    # ── detail panel for selected ─────────────────────────────────────────────
    a = apps[selected]
    detail_y = ctx.height - 52 - 30
    ctx.rect(0, detail_y, ctx.width, 52, fill=THEME.surface)
    ctx.text(PAD, detail_y + 10, a["description"] or a["name"],
             size=BODY, color=THEME.fg)
    ctx.text_right(ctx.width - PAD, detail_y + 10,
                   f"id: {a['id']}", size=CAPTION, color=THEME.muted)

    # ── status bar ────────────────────────────────────────────────────────────
    ctx.status_bar([
        ("j/k", "navigate"),
        ("r", "reload"),
        ("⌘W", "close"),
    ])


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


app.run()

#!/usr/bin/env python3
"""
parallax — Plexi viewer app for Parallax video projects.

View-only dashboard. Watches `.parallax/` under the launch directory and
re-renders when the manifest changes. Supports both layouts produced by the
Parallax CLI:

  parallax create  → flat manifest at  .parallax/manifest.yaml
                     stills at         stills/
                     output at         output/

  parallax run     → concept manifest at  .parallax/<concept_id>/manifest.yaml
                     stills at            .parallax/<concept_id>/stills/  (and *.png)
                     output at            .parallax/<concept_id>/output/

Never takes input beyond scroll / refresh — the chat lives in the linked
companion terminal pane declared by [app.launch] in manifest.toml.
"""
from __future__ import annotations

import glob
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":       "#1e1e2e",
    "surface":  "#313244",
    "text":     "#cdd6f4",
    "subtext":  "#9399b2",
    "muted":    "#6c7086",
    "accent":   "#89b4fa",
    "green":    "#a6e3a1",
    "peach":    "#fab387",
    "header":   "#181825",
}

PADDING = 16
HEADER_H = 56

VIDEO_EXTS = ("mp4", "mov", "webm", "mkv", "m4v")
IMAGE_EXTS = ("png", "jpg", "jpeg", "webp", "gif")

# ---------------------------------------------------------------------------
# Project state — re-read from disk on manifest mtime change
# ---------------------------------------------------------------------------

LAUNCH_DIR = os.getcwd()

_manifest_path: str = ""   # resolved dynamically by _poll_manifest()
_last_mtime: float = 0.0
_last_poll: float = 0.0
_manifest: dict = {}       # parsed manifest dict
_manifest_error: str = ""
_video_rect: tuple = (0, 0, 0, 0)  # (x, y, w, h) of latest video thumbnail

# ---------------------------------------------------------------------------
# Minimal YAML subset parser (stdlib only, no PyYAML dep).
#
# Supported schema only — this is NOT a general YAML parser:
#   project: "name"
#   scenes:
#     - number: 1
#       title: "..."
#       duration: 3.5
#     - number: 2
#       ...
# Strings may be quoted or bare. Numbers parse as float.
# ---------------------------------------------------------------------------


def _parse_scalar(s: str):
    s = s.strip()
    if not s:
        return None
    if (s.startswith('"') and s.endswith('"')) or (s.startswith("'") and s.endswith("'")):
        return s[1:-1]
    try:
        if "." in s:
            return float(s)
        return int(s)
    except ValueError:
        return s


def _parse_manifest_yaml(text: str) -> dict:
    """Parse the restricted Parallax manifest schema. Returns {} on failure."""
    result: dict = {"scenes": []}
    lines = [ln.rstrip() for ln in text.splitlines() if ln.strip() and not ln.lstrip().startswith("#")]
    in_scenes = False
    current: dict = {}
    for ln in lines:
        stripped = ln.lstrip()
        indent = len(ln) - len(stripped)
        # Top-level key: value
        if indent == 0 and ":" in stripped:
            key, _, rest = stripped.partition(":")
            key = key.strip()
            rest = rest.strip()
            if key == "scenes":
                in_scenes = True
                current = {}
                continue
            in_scenes = False
            if rest:
                result[key] = _parse_scalar(rest)
            continue
        # Inside scenes list
        if in_scenes:
            if stripped.startswith("- "):
                if current:
                    result["scenes"].append(current)
                current = {}
                stripped = stripped[2:]
                if ":" in stripped:
                    k, _, v = stripped.partition(":")
                    current[k.strip()] = _parse_scalar(v)
                continue
            if ":" in stripped:
                k, _, v = stripped.partition(":")
                current[k.strip()] = _parse_scalar(v)
    if current and in_scenes:
        result["scenes"].append(current)
    return result


def _resolve_manifest_path() -> str:
    """
    Find the most recently modified manifest.yaml under .parallax/.

    Checks two layouts:
      - .parallax/manifest.yaml          (parallax create / flat)
      - .parallax/<concept_id>/manifest.yaml  (parallax run)

    Returns the path with the highest mtime, or "" if none exist.
    """
    candidates = []

    flat = os.path.join(LAUNCH_DIR, ".parallax", "manifest.yaml")
    if os.path.isfile(flat):
        try:
            candidates.append((os.path.getmtime(flat), flat))
        except OSError:
            pass

    for p in glob.glob(os.path.join(LAUNCH_DIR, ".parallax", "*", "manifest.yaml")):
        try:
            candidates.append((os.path.getmtime(p), p))
        except OSError:
            pass

    if not candidates:
        return ""
    candidates.sort(reverse=True)
    return candidates[0][1]


def _reload_manifest(path: str) -> None:
    """Re-read the manifest file at *path*, resetting cached parse state."""
    global _manifest, _manifest_error
    try:
        with open(path, "r", encoding="utf-8") as f:
            text = f.read()
        _manifest = _parse_manifest_yaml(text)
        _manifest_error = ""
    except FileNotFoundError:
        _manifest = {}
        _manifest_error = ""  # treated specially — shows onboarding hint
    except Exception as e:  # parse / IO error
        _manifest = {}
        _manifest_error = f"manifest parse error: {e}"


def _poll_manifest() -> None:
    """
    Resolve the best manifest path once per second; reload if path or mtime
    changed.
    """
    global _manifest_path, _last_mtime, _last_poll
    now = time.monotonic()
    if now - _last_poll < 1.0:
        return
    _last_poll = now

    resolved = _resolve_manifest_path()

    if not resolved:
        # No manifest anywhere — reset if we previously had one.
        if _manifest_path or _last_mtime != 0.0:
            _manifest_path = ""
            _last_mtime = 0.0
            _reload_manifest("")
        return

    try:
        mtime = os.stat(resolved).st_mtime
    except OSError:
        mtime = 0.0

    if resolved != _manifest_path or mtime != _last_mtime:
        _manifest_path = resolved
        _last_mtime = mtime
        _reload_manifest(resolved)


# ---------------------------------------------------------------------------
# Filesystem helpers
# ---------------------------------------------------------------------------


def _project_dir() -> str:
    """
    Return the directory that contains stills and output for the current
    manifest.  Falls back to LAUNCH_DIR when no manifest is loaded.

    Layout rules:
      flat manifest  (.parallax/manifest.yaml)   → LAUNCH_DIR
      concept subdir (.parallax/<id>/manifest.yaml) → .parallax/<id>/
    """
    if not _manifest_path:
        return LAUNCH_DIR

    parallax_dir = os.path.join(LAUNCH_DIR, ".parallax")
    manifest_dir = os.path.dirname(_manifest_path)

    # Flat layout: manifest lives directly in .parallax/
    if os.path.normpath(manifest_dir) == os.path.normpath(parallax_dir):
        return LAUNCH_DIR

    # Concept subdir layout
    return manifest_dir


def _list_stills() -> list:
    proj = _project_dir()

    # Prefer a dedicated stills/ subdirectory.
    stills_dir = os.path.join(proj, "stills")
    if os.path.isdir(stills_dir):
        out = []
        for name in sorted(os.listdir(stills_dir)):
            if "." not in name:
                continue
            ext = name.rsplit(".", 1)[-1].lower()
            if ext in IMAGE_EXTS:
                out.append(os.path.join(stills_dir, name))
        if out:
            return out

    # Fallback: loose image files directly in the project dir (TEST_MODE stills).
    out = []
    try:
        for name in sorted(os.listdir(proj)):
            if "." not in name:
                continue
            ext = name.rsplit(".", 1)[-1].lower()
            if ext in IMAGE_EXTS:
                out.append(os.path.join(proj, name))
    except OSError:
        pass
    return out


def _latest_output() -> str | None:
    out_dir = os.path.join(_project_dir(), "output")
    if not os.path.isdir(out_dir):
        return None
    candidates = []
    for name in os.listdir(out_dir):
        if "." not in name:
            continue
        ext = name.rsplit(".", 1)[-1].lower()
        if ext in VIDEO_EXTS:
            path = os.path.join(out_dir, name)
            try:
                candidates.append((os.path.getmtime(path), path))
            except OSError:
                pass
    if not candidates:
        return None
    candidates.sort(reverse=True)
    return candidates[0][1]


# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------

app = App(app_id="parallax")


@app.on_render
def render(ctx):
    _poll_manifest()

    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    project_name = os.path.basename(os.path.normpath(LAUNCH_DIR)) or "parallax"
    scenes = _manifest.get("scenes") or []
    total_duration = 0.0
    for scene in scenes:
        d = scene.get("duration")
        if isinstance(d, (int, float)):
            total_duration += float(d)

    # --- Header ---------------------------------------------------------
    ctx.rect(0, 0, ctx.width, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 14, f"Parallax — {project_name}",
             size=15, color=C["accent"], bold=True)

    status = f"{len(scenes)} scenes"
    if total_duration > 0:
        status += f"   {total_duration:.1f}s"
    ctx.text(PADDING, 34, status, size=12, color=C["subtext"])

    hint = LAUNCH_DIR
    hint_x = ctx.width - len(hint) * 6.5 - PADDING
    if hint_x > PADDING + 220:
        ctx.text(hint_x, 20, hint, size=11, color=C["muted"], monospace=True)

    ctx.line(0, HEADER_H, ctx.width, HEADER_H, color=C["surface"], width=1.0)

    # --- Empty-state (no manifest yet) ----------------------------------
    if not _manifest_path:
        _render_empty(ctx)
        return

    if _manifest_error:
        ctx.text(PADDING, HEADER_H + 20,
                 _manifest_error, size=13, color=C["peach"])
        return

    # --- Main layout ----------------------------------------------------
    # Three vertical sections below the header:
    #   stills grid (top ~45%), video preview (middle ~30%),
    #   scene list (bottom ~25%)
    avail_h = ctx.height - HEADER_H - PADDING
    grid_h = max(120.0, avail_h * 0.45)
    video_h = max(120.0, avail_h * 0.30)
    list_h = max(80.0, avail_h - grid_h - video_h - PADDING * 2)

    y = HEADER_H + PADDING

    # --- Stills grid ----------------------------------------------------
    stills = _list_stills()
    ctx.text(PADDING, y, f"Stills ({len(stills)})",
             size=12, color=C["subtext"], bold=True)
    y += 18
    if stills:
        ctx.file_grid(
            x=PADDING,
            y=y,
            w=ctx.width - PADDING * 2,
            h=grid_h,
            paths=stills,
            item_size=140.0,
            show_labels=False,
        )
    else:
        ctx.text(PADDING, y + 8, "(no stills generated yet)",
                 size=12, color=C["muted"])
    y += grid_h + PADDING

    # --- Latest output preview ------------------------------------------
    latest = _latest_output()
    ctx.text(PADDING, y, "Latest output",
             size=12, color=C["subtext"], bold=True)
    y += 18
    if latest:
        global _video_rect
        vw = ctx.width - PADDING * 2
        _video_rect = (PADDING, y, vw, video_h)
        ctx.video_thumbnail(
            path=latest,
            x=PADDING,
            y=y,
            w=vw,
            h=video_h,
            show_play_button=True,
        )
        label = os.path.basename(latest)
        ctx.text(PADDING, y + video_h - 18, label,
                 size=11, color=C["text"], monospace=True)
    else:
        ctx.text(PADDING, y + 8, "(no rendered video yet)",
                 size=12, color=C["muted"])
    y += video_h + PADDING

    # --- Scene list -----------------------------------------------------
    ctx.text(PADDING, y, "Scenes",
             size=12, color=C["subtext"], bold=True)
    y += 18
    if scenes:
        items = []
        for scene in scenes:
            num = scene.get("number", "?")
            title = scene.get("title", "(untitled)")
            dur = scene.get("duration")
            secondary = None
            if isinstance(dur, (int, float)):
                secondary = f"{float(dur):.1f}s"
            items.append({
                "label": f"Scene {num}: {title}",
                "secondary": secondary,
            })
        # Clamp item_height so the list doesn't overflow.
        item_h = min(32.0, max(20.0, list_h / max(1, len(items))))
        ctx.list(items, selected=0, item_height=item_h)
    else:
        ctx.text(PADDING, y + 8, "(no scenes in manifest)",
                 size=12, color=C["muted"])


def _render_empty(ctx):
    w = ctx.width
    msg_y = HEADER_H + 40
    ctx.text(PADDING, msg_y,
             "No project here yet.",
             size=16, color=C["text"], bold=True)
    ctx.text(PADDING, msg_y + 28,
             "Describe what you want to create in the terminal below.",
             size=13, color=C["subtext"])

    ctx.text(PADDING, msg_y + 58,
             "Run a footage edit:",
             size=12, color=C["muted"])
    cmd = '  parallax run "your brief"'
    ctx.rect(PADDING, msg_y + 76, w - PADDING * 2, 32,
             fill=C["surface"], radius=6.0)
    ctx.text(PADDING + 12, msg_y + 84, cmd,
             size=12, color=C["green"], monospace=True)

    ctx.text(PADDING, msg_y + 122,
             "Or scaffold a new project directory:",
             size=12, color=C["muted"])
    cmd2 = '  parallax project new <name>'
    ctx.rect(PADDING, msg_y + 140, w - PADDING * 2, 32,
             fill=C["surface"], radius=6.0)
    ctx.text(PADDING + 12, msg_y + 148, cmd2,
             size=12, color=C["green"], monospace=True)


@app.on_mouse_down
def on_mouse_down(x, y, button, emit):
    if button != "left":
        return
    vx, vy, vw, vh = _video_rect
    if vx <= x <= vx + vw and vy <= y <= vy + vh:
        path = _latest_output()
        if path:
            import subprocess
            try:
                subprocess.Popen(["open", path])
            except Exception as e:
                emit.warn(f"could not open video: {e}")


app.run()

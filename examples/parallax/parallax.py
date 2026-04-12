#!/usr/bin/env python3
"""
parallax — Plexi viewer app for Parallax video projects.

View-only dashboard. Watches `.parallax/manifest.yaml` under the launch
directory and re-renders when the file changes. Never takes input beyond
scroll / refresh — the chat lives in the linked companion terminal pane
declared by [app.launch] in manifest.toml.

Layout:
  Header       — project name, scene count, total duration
  File grid    — generated stills from stills/
  Video        — latest output from output/ (video_thumbnail)
  Scene list   — scenes from the manifest

If the manifest is missing, renders a friendly "run `parallax run` in the
terminal below" hint.
"""

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
MANIFEST_PATH = os.path.join(LAUNCH_DIR, ".parallax", "manifest.yaml")

_last_mtime: float = 0.0
_last_poll: float = 0.0
_manifest: dict = {}  # parsed manifest dict
_manifest_error: str = ""

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


def _reload_manifest():
    """Re-read the manifest file, resetting cached parse state."""
    global _manifest, _manifest_error
    try:
        with open(MANIFEST_PATH, "r", encoding="utf-8") as f:
            text = f.read()
        _manifest = _parse_manifest_yaml(text)
        _manifest_error = ""
    except FileNotFoundError:
        _manifest = {}
        _manifest_error = ""  # treated specially — shows onboarding hint
    except Exception as e:  # parse / IO error
        _manifest = {}
        _manifest_error = f"manifest parse error: {e}"


def _poll_manifest():
    """Check manifest mtime once per second; reload on change."""
    global _last_mtime, _last_poll
    now = time.monotonic()
    if now - _last_poll < 1.0:
        return
    _last_poll = now
    try:
        st = os.stat(MANIFEST_PATH)
        mtime = st.st_mtime
    except FileNotFoundError:
        if _last_mtime != 0.0:
            _last_mtime = 0.0
            _reload_manifest()
        return
    if mtime != _last_mtime:
        _last_mtime = mtime
        _reload_manifest()


# ---------------------------------------------------------------------------
# Filesystem helpers
# ---------------------------------------------------------------------------


def _list_stills() -> list[str]:
    stills_dir = os.path.join(LAUNCH_DIR, "stills")
    if not os.path.isdir(stills_dir):
        return []
    out = []
    for name in sorted(os.listdir(stills_dir)):
        if "." not in name:
            continue
        ext = name.rsplit(".", 1)[-1].lower()
        if ext in IMAGE_EXTS:
            out.append(os.path.join(stills_dir, name))
    return out


def _latest_output() -> str | None:
    out_dir = os.path.join(LAUNCH_DIR, "output")
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
    if not os.path.exists(MANIFEST_PATH):
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
        ctx.text(PADDING, y + 8, "(no stills generated yet — stills/ is empty)",
                 size=12, color=C["muted"])
    y += grid_h + PADDING

    # --- Latest output preview ------------------------------------------
    latest = _latest_output()
    ctx.text(PADDING, y, "Latest output",
             size=12, color=C["subtext"], bold=True)
    y += 18
    if latest:
        ctx.video_thumbnail(
            path=latest,
            x=PADDING,
            y=y,
            w=ctx.width - PADDING * 2,
            h=video_h,
            show_play_button=True,
        )
        label = os.path.basename(latest)
        ctx.text(PADDING, y + video_h - 18, label,
                 size=11, color=C["text"], monospace=True)
    else:
        ctx.text(PADDING, y + 8, "(no rendered video yet — output/ is empty)",
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
             "No Parallax project here yet.",
             size=16, color=C["text"], bold=True)
    ctx.text(PADDING, msg_y + 28,
             f"Expected: {MANIFEST_PATH}",
             size=11, color=C["muted"], monospace=True)
    ctx.text(PADDING, msg_y + 58,
             "Run this in the terminal below to start one:",
             size=13, color=C["subtext"])

    cmd = '  parallax run "your brief here"'
    ctx.rect(PADDING, msg_y + 80, w - PADDING * 2, 32,
             fill=C["surface"], radius=6.0)
    ctx.text(PADDING + 12, msg_y + 88, cmd,
             size=13, color=C["green"], monospace=True)


app.run()

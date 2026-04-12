#!/usr/bin/env python3
"""
app-store — Plexi app store
Browse and install Plexi apps from the community registry.
"""
from __future__ import annotations

import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # type: ignore[import]

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":       "#1e1e2e",
    "surface":  "#313244",
    "overlay":  "#45475a",
    "text":     "#cdd6f4",
    "subtext":  "#6c7086",
    "accent":   "#89b4fa",   # not installed
    "installed":"#a6e3a1",   # installed
    "dimmed":   "#45475a",
    "header":   "#181825",
    "red":      "#f38ba8",
    "yellow":   "#f9e2af",
    "green":    "#a6e3a1",
}

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

REGISTRY_URL  = "https://raw.githubusercontent.com/ianjamesburke/plexi-registry/main/registry.json"
CACHE_PATH    = pathlib.Path.home() / ".plexi-alpha" / "app_store_cache.json"
APPS_DIR      = pathlib.Path.home() / ".plexi-alpha" / "apps"
CACHE_TTL_S   = 3600  # 1 hour

HEADER_H      = 32.0
FILTER_H      = 32.0
ROW_H         = 52.0
DETAIL_PADDING = 20.0

TAG_CYCLE = ["all", "game", "productivity", "creative", "system"]

SPINNER_FRAMES = ["|", "/", "-", "\\"]
SPINNER_FPS    = 8

VIEW_BROWSE  = "browse"
VIEW_DETAIL  = "detail"
VIEW_INSTALL = "install"

class State:
    def __init__(self):
        self.view = VIEW_BROWSE

        # Registry
        self.registry: list[dict] = []
        self.loading = True
        self.load_error: str | None = None

        # Browse
        self.cursor = 0
        self.scroll_offset = 0.0
        self.filter_active = False
        self.filter_text = ""
        self.filter_cursor_blink = True
        self.tag_filter_idx = 0  # index into TAG_CYCLE

        # Detail / install
        self.selected_entry: dict | None = None
        self.confirm_uninstall = False

        # Install
        self.install_status = ""
        self.install_error: str | None = None
        self.install_done = False
        self.install_done_name = ""
        self.install_done_time = 0.0
        self._install_thread: threading.Thread | None = None

        # Update All
        self.update_all_status: str = ""
        self.update_all_running = False

        # Timing
        self._start = time.monotonic()

    @property
    def elapsed(self) -> float:
        return time.monotonic() - self._start

    def filtered_entries(self) -> list[dict]:
        entries = self.registry
        tag_filter = TAG_CYCLE[self.tag_filter_idx]
        if tag_filter != "all":
            entries = [e for e in entries if tag_filter in e.get("tags", [])]
        q = self.filter_text.lower().strip()
        if q:
            def match(e):
                return (
                    q in e.get("name", "").lower()
                    or q in e.get("description", "").lower()
                    or any(q in t.lower() for t in e.get("tags", []))
                )
            entries = [e for e in entries if match(e)]
        return entries

    def clamp_cursor(self, entries):
        if entries:
            self.cursor = max(0, min(self.cursor, len(entries) - 1))
        else:
            self.cursor = 0


state = State()

def _load_registry():
    try:
        if CACHE_PATH.exists():
            raw = CACHE_PATH.read_text()
            cached = json.loads(raw)
            age = time.time() - cached.get("timestamp", 0)
            if age < CACHE_TTL_S:
                state.registry = cached.get("apps", [])
                state.loading = False
                return
    except Exception:
        pass

    # Fetch from network
    try:
        with urllib.request.urlopen(REGISTRY_URL, timeout=10) as resp:
            data = json.loads(resp.read().decode("utf-8"))
        apps = data if isinstance(data, list) else data.get("apps", [])
        state.registry = apps
        # Write cache
        try:
            CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
            CACHE_PATH.write_text(json.dumps({"timestamp": time.time(), "apps": apps}))
        except Exception:
            pass
        state.loading = False
        return
    except Exception as fetch_err:
        try:
            if CACHE_PATH.exists():
                raw = CACHE_PATH.read_text()
                cached = json.loads(raw)
                state.registry = cached.get("apps", [])
                state.loading = False
                return
        except Exception:
            pass
        state.load_error = f"Unable to load registry: {fetch_err}"
        state.loading = False


threading.Thread(target=_load_registry, daemon=True).start()
def _parse_entry_from_manifest(manifest_text: str) -> str | None:
    try:
        import tomllib  # type: ignore
        data = tomllib.loads(manifest_text)
        return data.get("entry") or data.get("app", {}).get("entry")
    except ImportError:
        pass
    try:
        import tomli  # type: ignore
        data = tomli.loads(manifest_text)
        return data.get("entry") or data.get("app", {}).get("entry")
    except ImportError:
        pass
    # Regex fallback
    m = re.search(r'entry\s*=\s*["\']([^"\']+)["\']', manifest_text)
    if m:
        return m.group(1)
    return None


def _fallback_install(entry: dict, app_dir: pathlib.Path) -> None:
    repo = entry.get("repo", "")
    path = entry.get("path", "")
    branch = entry.get("branch", "alpha")
    base_url = f"https://raw.githubusercontent.com/{repo}/{branch}/{path}"
    manifest_url = f"{base_url}/manifest.toml"
    try:
        with urllib.request.urlopen(manifest_url, timeout=10) as r:
            manifest_text = r.read().decode("utf-8")
    except Exception as e:
        raise RuntimeError(f"Failed to fetch manifest: {e}") from e

    entry_file = _parse_entry_from_manifest(manifest_text)
    if not entry_file:
        raise RuntimeError("Could not determine entry filename from manifest")

    app_dir.mkdir(parents=True, exist_ok=True)
    (app_dir / "manifest.toml").write_text(manifest_text)

    entry_url = f"{base_url}/{entry_file}"
    try:
        with urllib.request.urlopen(entry_url, timeout=10) as r:
            entry_content = r.read()
        entry_path = app_dir / entry_file
        entry_path.write_bytes(entry_content)
        os.chmod(entry_path, 0o755)
    except Exception as e:
        raise RuntimeError(f"Failed to fetch entry file: {e}") from e

    sdk_url = f"{base_url}/plexi_sdk.py"
    try:
        with urllib.request.urlopen(sdk_url, timeout=10) as r:
            (app_dir / "plexi_sdk.py").write_bytes(r.read())
    except Exception:
        try:
            our_sdk = pathlib.Path(__file__).parent / "plexi_sdk.py"
            if our_sdk.exists():
                shutil.copy(our_sdk, app_dir / "plexi_sdk.py")
        except Exception:
            pass


def _do_install(entry: dict):
    app_id = entry.get("id", "unknown")
    app_dir = APPS_DIR / app_id
    name = entry.get("name", app_id)

    state.install_status = f"Installing {name}..."
    state.install_error = None
    state.install_done = False

    # Attempt 1: CLI
    try:
        repo = entry.get("repo", "")
        result = subprocess.run(
            ["plexi-alpha", "app", "install", f"{repo}/{app_id}"],
            capture_output=True, timeout=30,
        )
        if result.returncode == 0:
            state.install_done = True
            state.install_done_name = name
            state.install_done_time = time.monotonic()
            state.install_status = f"Installed! Reload Plexi to use {name}."
            return
    except Exception:
        pass

    # Attempt 2: Fallback direct download
    try:
        _fallback_install(entry, app_dir)
        state.install_done = True
        state.install_done_name = name
        state.install_done_time = time.monotonic()
        state.install_status = f"Installed! Reload Plexi to use {name}."
    except Exception as e:
        state.install_error = str(e)
        state.install_status = f"Install failed: {e}"


def start_install(entry: dict):
    state.view = VIEW_INSTALL
    state.install_done = False
    state.install_error = None
    state.install_status = ""
    t = threading.Thread(target=_do_install, args=(entry,), daemon=True)
    state._install_thread = t
    t.start()


def _do_update_all(entries_with_updates: list[dict]):
    n = len(entries_with_updates)
    state.update_all_status = f"Updating {n} app{'s' if n != 1 else ''}…"
    state.update_all_running = True
    for i, entry in enumerate(entries_with_updates):
        name = entry.get("name", entry.get("id", "?"))
        state.update_all_status = f"Updating {name} ({i + 1}/{n})…"
        app_id = entry.get("id", "unknown")
        app_dir = APPS_DIR / app_id
        try:
            repo = entry.get("repo", "")
            result = subprocess.run(
                ["plexi-alpha", "app", "install", f"{repo}/{app_id}"],
                capture_output=True, timeout=30,
            )
            if result.returncode != 0:
                _fallback_install(entry, app_dir)
        except Exception:
            try:
                _fallback_install(entry, app_dir)
            except Exception:
                pass
    state.update_all_status = f"Updated {n} app{'s' if n != 1 else ''}."
    state.update_all_running = False
    # Clear the status message after a short delay
    def _clear():
        time.sleep(3)
        state.update_all_status = ""
    threading.Thread(target=_clear, daemon=True).start()


def start_update_all():
    updatable = [e for e in state.registry if is_installed(e.get("id", "")) and _has_update(e)]
    if not updatable:
        return
    t = threading.Thread(target=_do_update_all, args=(updatable,), daemon=True)
    t.start()


def uninstall_app(app_id: str):
    try:
        shutil.rmtree(APPS_DIR / app_id)
    except Exception:
        pass


def is_installed(app_id: str) -> bool:
    return (APPS_DIR / app_id).exists()


def _installed_version(app_id: str) -> str | None:
    """Read version from installed manifest.toml, return None if not installed."""
    manifest = pathlib.Path.home() / ".plexi-alpha" / "apps" / app_id / "manifest.toml"
    try:
        text = manifest.read_text()
        for line in text.splitlines():
            if line.strip().startswith("version"):
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    except Exception:
        pass
    return None


def _has_update(entry: dict) -> bool:
    """Return True if registry version > installed version."""
    app_id = entry.get("id", "")
    registry_ver = entry.get("version", "")
    installed_ver = _installed_version(app_id)
    if not installed_ver or not registry_ver:
        return False
    return registry_ver != installed_ver


def truncate(text: str, max_chars: int) -> str:
    if len(text) <= max_chars:
        return text
    return text[:max_chars - 1] + "…"


def visible_rows(pane_h: float) -> int:
    usable = pane_h - HEADER_H - (FILTER_H if state.filter_active else 0)
    return max(1, int(usable // ROW_H))


def render_header(ctx, title: str, subtitle: str = ""):
    ctx.rect(0, 0, ctx.width, HEADER_H, fill=C["header"])
    ctx.text(12, 9, title, size=14, color=C["text"], bold=True)
    if subtitle:
        sub_x = ctx.width - len(subtitle) * 7 - 12
        ctx.text(max(200, sub_x), 10, subtitle, size=12, color=C["subtext"])


def render_tag_pill(ctx, tag: str, x: float, y: float):
    w = len(tag) * 7 + 10
    ctx.rect(x, y, w, 16, fill=C["surface"], radius=4.0)
    ctx.text(x + 5, y + 2, tag, size=10, color=C["subtext"])
    return w + 6


def render_browse(ctx, now: float):
    entries = state.filtered_entries()
    state.clamp_cursor(entries)

    n = len(entries)
    tag_label = TAG_CYCLE[state.tag_filter_idx]
    tag_str = f"[{tag_label}]"
    subtitle_parts = []
    if n > 0:
        subtitle_parts.append(f"{n} apps")
    if tag_label != "all":
        subtitle_parts.append(tag_str)
    subtitle = "  ".join(subtitle_parts) if subtitle_parts else ""
    render_header(ctx, "App Store", subtitle)

    if state.loading:
        ctx.text(12, HEADER_H + 20, "Loading registry...", size=13, color=C["subtext"])
        return

    if state.load_error:
        ctx.text(12, HEADER_H + 20, state.load_error, size=13, color=C["red"])
        return

    if not entries:
        msg = "No apps match." if state.filter_text or tag_label != "all" else "Registry is empty."
        ctx.text(12, HEADER_H + 20, msg, size=13, color=C["subtext"])
    else:
        vis = visible_rows(ctx.height)

        # Keep cursor in scroll window
        if state.cursor < int(state.scroll_offset):
            state.scroll_offset = float(state.cursor)
        elif state.cursor >= int(state.scroll_offset) + vis:
            state.scroll_offset = float(state.cursor - vis + 1)

        for i in range(vis):
            idx = int(state.scroll_offset) + i
            if idx >= n:
                break
            entry = entries[idx]
            app_id = entry.get("id", "")
            name = entry.get("name", app_id)
            desc = entry.get("description", "")
            tags = entry.get("tags", [])
            installed = is_installed(app_id)

            ry = HEADER_H + i * ROW_H

            if idx == state.cursor:
                ctx.rect(0, ry, ctx.width, ROW_H, fill=C["surface"])
                ctx.rect(0, ry, 3, ROW_H, fill=C["accent"])

            name_color = C["dimmed"] if installed else C["accent"]
            ctx.text(12, ry + 9, name, size=13, color=name_color, bold=True)

            max_desc_chars = max(10, int((ctx.width - 24 - 90) / 7))
            ctx.text(12, ry + 28, truncate(desc, max_desc_chars), size=11, color=C["subtext"])

            # Right-side installed / update badge
            if installed:
                if _has_update(entry):
                    badge = "update"
                    badge_color = C["yellow"]
                else:
                    badge = "installed"
                    badge_color = C["installed"]
                bx = ctx.width - len(badge) * 7 - 16
                ctx.rect(bx - 4, ry + 14, len(badge) * 7 + 8, 16, fill=C["surface"], radius=4.0)
                ctx.text(bx, ry + 16, badge, size=10, color=badge_color)

            # Tag pills (first tag only, to keep rows clean)
            if tags:
                tx = ctx.width - len(tags[0]) * 7 - 20
                if not installed:
                    render_tag_pill(ctx, tags[0], tx, ry + 14)

    # Filter bar at bottom
    if state.filter_active:
        fy = ctx.height - FILTER_H
        ctx.rect(0, fy, ctx.width, FILTER_H, fill=C["surface"])
        prompt = "/"
        query_display = state.filter_text
        # Blinking cursor
        blink = int(now * 2) % 2 == 0
        if blink:
            query_display += "|"
        ctx.text(12, fy + 8, f"{prompt} {query_display}", size=13, color=C["text"])

    # Update All status message (shown in header area while running)
    if state.update_all_status:
        status_x = ctx.width // 2 - len(state.update_all_status) * 3
        ctx.text(max(120, status_x), 9, state.update_all_status, size=11, color=C["yellow"])

    # Hint bar (only when filter not active)
    if not state.filter_active:
        hints = "j/k navigate  /  filter  Tab  tag  Enter  open  U  update all"
        ctx.text(12, ctx.height - 18, hints, size=10, color=C["dimmed"])


def render_detail(ctx, _now: float):
    if not state.selected_entry:
        state.view = VIEW_BROWSE
        return

    entry = state.selected_entry
    app_id = entry.get("id", "")
    name = entry.get("name", app_id)
    author = entry.get("author", "unknown")
    version = entry.get("version", "")
    tags = entry.get("tags", [])
    desc = entry.get("description", "")
    installed = is_installed(app_id)

    render_header(ctx, name, f"v{version}" if version else "")

    y = HEADER_H + DETAIL_PADDING

    # Author
    ctx.text(DETAIL_PADDING, y, f"by {author}", size=12, color=C["subtext"])
    y += 22

    # Tags row
    tx = DETAIL_PADDING
    for tag in tags:
        w = render_tag_pill(ctx, tag, tx, y)
        tx += w
    if tags:
        y += 26

    # Description (word-wrapped)
    words = desc.split()
    line = ""
    max_w = max(10, int((ctx.width - DETAIL_PADDING * 2) / 7))
    for word in words:
        if len(line) + len(word) + 1 <= max_w:
            line = (line + " " + word).strip()
        else:
            if line:
                ctx.text(DETAIL_PADDING, y, line, size=13, color=C["text"])
                y += 20
            line = word
    if line:
        ctx.text(DETAIL_PADDING, y, line, size=13, color=C["text"])
        y += 28

    # Install status
    if installed:
        has_upd = _has_update(entry)
        registry_ver = entry.get("version", "")
        if has_upd:
            ctx.text(DETAIL_PADDING, y, f"Update available (v{registry_ver})", size=13, color=C["yellow"])
        else:
            ctx.text(DETAIL_PADDING, y, "Already installed", size=13, color=C["installed"])
        y += 24
        if state.confirm_uninstall:
            ctx.text(DETAIL_PADDING, y, f"Uninstall {name}? (y/n)", size=13, color=C["yellow"])
        elif has_upd:
            ctx.text(DETAIL_PADDING, y, "u  update    x  uninstall    q / Backspace  back", size=11, color=C["subtext"])
        else:
            ctx.text(DETAIL_PADDING, y, "x  uninstall    q / Backspace  back", size=11, color=C["subtext"])
    else:
        ctx.text(DETAIL_PADDING, y, "Not installed", size=13, color=C["subtext"])
        y += 24
        ctx.text(DETAIL_PADDING, y, "i  install    q / Backspace  back", size=11, color=C["subtext"])


def render_install(ctx, now: float):
    if not state.selected_entry:
        state.view = VIEW_BROWSE
        return

    name = state.selected_entry.get("name", "")
    render_header(ctx, f"Installing {name}")

    cx = ctx.width / 2
    cy = ctx.height / 2

    if state.install_done:
        ctx.text(cx - 160, cy - 10, state.install_status, size=14, color=C["installed"])
        ctx.text(cx - 100, cy + 20, "Backspace / q  to go back", size=11, color=C["subtext"])
    elif state.install_error:
        ctx.text(cx - 160, cy - 10, state.install_status, size=13, color=C["red"])
        ctx.text(cx - 100, cy + 20, "Backspace / q  to go back", size=11, color=C["subtext"])
    else:
        frame_idx = int(now * SPINNER_FPS) % len(SPINNER_FRAMES)
        spinner = SPINNER_FRAMES[frame_idx]
        ctx.text(cx - 10, cy - 10, spinner, size=20, color=C["accent"], monospace=True)
        ctx.text(cx - 100, cy + 20, state.install_status or "Working...", size=13, color=C["text"])


app = App(app_id="app-store")


@app.on_render
def render(ctx):
    now = state.elapsed
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    # Auto-return from install done after 3 seconds
    if state.view == VIEW_INSTALL and state.install_done:
        if time.monotonic() - state.install_done_time > 3.0:
            state.view = VIEW_DETAIL

    if state.view == VIEW_BROWSE:
        render_browse(ctx, now)
    elif state.view == VIEW_DETAIL:
        render_detail(ctx, now)
    elif state.view == VIEW_INSTALL:
        render_install(ctx, now)


@app.on_key
def on_key(key, _mods, _emit):
    # ------------------------------------------------------------------ install
    if state.view == VIEW_INSTALL:
        if key in ("Backspace", "q", "Escape") and (state.install_done or state.install_error):
            state.view = VIEW_DETAIL
            state.install_done = False
        return

    # ------------------------------------------------------------------ detail
    if state.view == VIEW_DETAIL:
        entry = state.selected_entry
        if not entry:
            state.view = VIEW_BROWSE
            return
        app_id = entry.get("id", "")

        if state.confirm_uninstall:
            if key == "y":
                uninstall_app(app_id)
                state.confirm_uninstall = False
            elif key in ("n", "Escape", "q", "Backspace"):
                state.confirm_uninstall = False
            return

        if key in ("Backspace", "q", "Escape"):
            state.view = VIEW_BROWSE
            state.confirm_uninstall = False
        elif key == "i" and not is_installed(app_id):
            start_install(entry)
        elif key == "u" and is_installed(app_id):
            # u = update (re-installs over existing)
            start_install(entry)
        elif key == "x" and is_installed(app_id):
            state.confirm_uninstall = True
        return

    # ------------------------------------------------------------------ browse
    entries = state.filtered_entries()

    if state.filter_active:
        if key == "Escape":
            state.filter_active = False
            state.filter_text = ""
        elif key == "Backspace":
            state.filter_text = state.filter_text[:-1]
            if not state.filter_text:
                state.filter_active = False
        elif key == "Enter":
            state.filter_active = False
            if entries:
                state.selected_entry = entries[state.cursor]
                state.view = VIEW_DETAIL
        elif len(key) == 1:
            state.filter_text += key
            state.cursor = 0
            state.scroll_offset = 0.0
        return

    # Normal browse
    if key in ("j", "ArrowDown"):
        state.cursor = min(state.cursor + 1, max(0, len(entries) - 1))
    elif key in ("k", "ArrowUp"):
        state.cursor = max(state.cursor - 1, 0)
    elif key == "/":
        state.filter_active = True
        state.filter_text = ""
        state.cursor = 0
        state.scroll_offset = 0.0
    elif key == "Escape":
        state.filter_text = ""
        state.cursor = 0
        state.scroll_offset = 0.0
    elif key == "Tab":
        state.tag_filter_idx = (state.tag_filter_idx + 1) % len(TAG_CYCLE)
        state.cursor = 0
        state.scroll_offset = 0.0
    elif key == "Enter" and entries:
        state.selected_entry = entries[state.cursor]
        state.view = VIEW_DETAIL
        state.confirm_uninstall = False
    elif key == "U" and not state.update_all_running:
        start_update_all()


@app.on_scroll
def on_scroll(x, y, delta_x, delta_y, _emit):
    if state.view == VIEW_BROWSE:
        entries = state.filtered_entries()
        n = len(entries)
        if n == 0:
            return
        state.scroll_offset = max(0.0, min(state.scroll_offset + delta_y * 0.1, float(n - 1)))
        state.cursor = int(state.scroll_offset)


app.run()

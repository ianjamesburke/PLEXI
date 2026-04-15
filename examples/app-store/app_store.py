from __future__ import annotations
"""
app-store — Plexi app store
Browse, install, and update Plexi apps from the community registry.

Each row shows the available version (and the installed version when an
update is available) plus a status badge:

    NEW        — not installed
    INSTALLED  — installed and up to date
    UPDATE     — installed, newer version available

Keys (browse view):
    j / k        navigate
    /            filter
    Tab          cycle tag filter
    Enter        open detail
    i            install (in detail view, when not installed)
    u            update the selected app in place (preserves user data files
                 like feedback.jsonl)
    U            update every app that has an UPDATE badge
    x            uninstall (in detail view)
"""

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
from plexi_sdk import (  # type: ignore[import]
    App, load_manifest,
    THEME,
    TITLE, HEADING, BODY, CAPTION, HINT,
    PAD, PAD_TIGHT, HEADER_H,
)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

REGISTRY_URL  = "https://raw.githubusercontent.com/ianjamesburke/plexi-registry/main/registry.json"
CACHE_PATH    = pathlib.Path.home() / ".plexi-alpha" / "app_store_cache.json"
APPS_DIR      = pathlib.Path.home() / ".plexi-alpha" / "apps"
CACHE_TTL_S   = 3600  # 1 hour

ROW_H         = 64.0

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
        self.filter_active = False
        self.filter_text = ""
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

        # Detail scroll offset (for long descriptions)
        self.detail_scroll = 0

        # Update All
        self.update_all_status: str = ""
        self.update_all_running = False

        # Transient inline error message (shown for ~3s)
        self.transient_error: str = ""
        self.transient_error_time: float = 0.0

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
    base_url = f"https://raw.githubusercontent.com/{repo}/{branch}/{path}".rstrip("/")
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


def _do_update(entry: dict, emit):
    """In-place update: re-fetch source files into the app dir, preserving
    any files (like feedback.jsonl) that exist locally but not in the source."""
    app_id = entry.get("id", "unknown")
    name = entry.get("name", app_id)
    app_dir = APPS_DIR / app_id
    old_ver = _installed_version(app_id) or "?"
    new_ver = entry.get("version", "?")
    try:
        # _fallback_install only writes the known source files (manifest.toml,
        # entry .py, plexi_sdk.py) — it does NOT rmtree the app dir, so any
        # user data files in app_dir are preserved automatically.
        _fallback_install(entry, app_dir)
        try:
            emit.info(f"updated {app_id} {old_ver} → {new_ver}")
        except Exception:
            pass
    except Exception as e:
        try:
            emit.error(f"update failed for {app_id}: {e}")
        except Exception:
            pass
        _set_transient_error(f"Update failed: {e}")


def start_update(entry: dict, emit):
    """Trigger an in-place update for an installed app on a background thread."""
    if not entry:
        return
    app_id = entry.get("id", "")
    if not is_installed(app_id):
        try:
            emit.info(f"update skipped: {app_id} is not installed")
        except Exception:
            pass
        return
    if not _has_update(entry):
        try:
            emit.info(f"update skipped: {app_id} is already at latest version")
        except Exception:
            pass
        return
    threading.Thread(target=_do_update, args=(entry, emit), daemon=True).start()


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


def _version_tuple(v: str) -> tuple[int, ...]:
    # Strip any pre-release / build suffix ("1.2.3-alpha", "1.2.3+meta") then
    # split on dots and coerce to ints. Non-numeric components become 0.
    if not v:
        return (0,)
    base = v.split("-", 1)[0].split("+", 1)[0]
    parts: list[int] = []
    for p in base.split("."):
        try:
            parts.append(int(p))
        except ValueError:
            parts.append(0)
    return tuple(parts) if parts else (0,)


def compare_versions(a: str, b: str) -> int:
    """Compare two version strings. Returns -1 if a<b, 0 if equal, 1 if a>b."""
    ta, tb = _version_tuple(a), _version_tuple(b)
    n = max(len(ta), len(tb))
    ta = ta + (0,) * (n - len(ta))
    tb = tb + (0,) * (n - len(tb))
    if ta < tb:
        return -1
    if ta > tb:
        return 1
    return 0


def _installed_version(app_id: str) -> str | None:
    """Read version from installed manifest.toml, return None if not installed."""
    manifest_path = APPS_DIR / app_id / "manifest.toml"
    if not manifest_path.exists():
        return None
    try:
        manifest = load_manifest(str(manifest_path))
        ver = manifest.get("app", {}).get("version", "")
        return ver or None
    except Exception:
        return None


def _has_update(entry: dict) -> bool:
    """Return True if registry version is strictly greater than installed version."""
    app_id = entry.get("id", "")
    registry_ver = entry.get("version", "") or "0.0.0"
    installed_ver = _installed_version(app_id)
    if not installed_ver:
        return False
    return compare_versions(registry_ver, installed_ver) > 0


def _set_transient_error(msg: str):
    state.transient_error = msg
    state.transient_error_time = time.monotonic()


def truncate(text: str, max_chars: int) -> str:
    if len(text) <= max_chars:
        return text
    return text[:max_chars - 1] + "…"


# ---------------------------------------------------------------------------
# Row renderer — browse list
# ---------------------------------------------------------------------------

def _render_entry_row(ctx, entry, i, x, y, w, is_sel):
    app_id = entry.get("id", "")
    name = entry.get("name", app_id)
    desc = entry.get("description", "")
    installed = is_installed(app_id)
    installed_ver = _installed_version(app_id) if installed else None
    available_ver = entry.get("version", "") or ""

    # Row background + selection accent.
    bg = THEME.highlight if is_sel else THEME.bg
    ctx.rect(x + PAD / 2, y, w - PAD, ROW_H - 4, fill=bg, radius=6)
    if is_sel:
        ctx.rect(x + PAD / 2, y, 3, ROW_H - 4, fill=THEME.accent, radius=2)

    # Name (left column).
    name_color = THEME.muted if installed else THEME.accent
    ctx.text(
        x + PAD + 8, y + 8, name,
        size=BODY, color=THEME.fg if is_sel else name_color, bold=True,
    )

    # Description (left column, second line), width-budgeted against the right badge area.
    desc_max_px = max(40.0, w - PAD * 2 - 200.0)
    desc_max_chars = max(10, int(desc_max_px / (CAPTION * 0.52)))
    ctx.text(
        x + PAD + 8, y + 8 + BODY + 6, truncate(desc, desc_max_chars),
        size=CAPTION, color=THEME.muted,
    )

    # Right-side badge.
    if installed:
        if _has_update(entry):
            badge = "UPDATE"
            badge_color = THEME.yellow
        else:
            badge = "INSTALLED"
            badge_color = THEME.green
    else:
        badge = "NEW"
        badge_color = THEME.accent

    badge_right = x + w - PAD
    ctx.text_right(badge_right, y + 8, badge, size=HINT, color=badge_color, bold=True)

    # Version line under the badge, right-aligned.
    if installed and installed_ver and available_ver and compare_versions(available_ver, installed_ver) > 0:
        ver_line = f"v{installed_ver} → v{available_ver}"
        ver_color = THEME.yellow
    elif installed and installed_ver:
        ver_line = f"v{installed_ver}"
        ver_color = THEME.muted
    elif available_ver:
        ver_line = f"v{available_ver}"
        ver_color = THEME.muted
    else:
        ver_line = ""
        ver_color = THEME.muted

    if ver_line:
        ctx.text_right(
            badge_right, y + 8 + HINT + 6, ver_line,
            size=HINT, color=ver_color,
        )


# ---------------------------------------------------------------------------
# Views
# ---------------------------------------------------------------------------

app = App(app_id="app-store")


def render_browse(ctx, now: float):
    entries = state.filtered_entries()
    state.clamp_cursor(entries)

    n = len(entries)
    tag_label = TAG_CYCLE[state.tag_filter_idx]

    # Subtitle: "N apps · [tag]" (when filtered to a specific tag).
    subtitle_parts = []
    if n > 0:
        subtitle_parts.append(f"{n} app{'s' if n != 1 else ''}")
    if tag_label != "all":
        subtitle_parts.append(f"[{tag_label}]")
    if state.filter_text:
        subtitle_parts.append(f"/{state.filter_text}")
    subtitle = "  ·  ".join(subtitle_parts) if subtitle_parts else None

    ctx.header("App Store", subtitle=subtitle)

    if state.loading:
        ctx.empty_state("Loading registry…", icon_color=THEME.muted)
        _browse_status_bar(ctx)
        return

    if state.load_error:
        ctx.empty_state(state.load_error, icon_color=THEME.red)
        _browse_status_bar(ctx)
        return

    if not entries:
        msg = "No apps match." if state.filter_text or tag_label != "all" else "Registry is empty."
        ctx.empty_state(msg, icon_color=THEME.muted)
        _browse_status_bar(ctx)
        return

    ctx.scrollable_list(
        list_id="apps",
        items=entries,
        selected=state.cursor,
        row_height=ROW_H,
        render_row=_render_entry_row,
    )

    _browse_status_bar(ctx)


def _browse_status_bar(ctx):
    # Transient error takes priority over the shortcut hints.
    if state.transient_error and (time.monotonic() - state.transient_error_time) < 3.0:
        ctx.status_bar([], status_msg=state.transient_error, status_color=THEME.red)
        return

    if state.update_all_status:
        ctx.status_bar([], status_msg=state.update_all_status, status_color=THEME.yellow)
        return

    if state.filter_active:
        ctx.status_bar(
            [("/", f"{state.filter_text}_"), ("Enter", "open"), ("Esc", "cancel")],
        )
        return

    ctx.status_bar(
        [
            ("j/k", "nav"),
            ("/", "filter"),
            ("Tab", "tag"),
            ("Enter", "open"),
            ("u/U", "update"),
            ("⌘W", "close"),
        ],
    )


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

    subtitle_bits: list[str] = []
    if version:
        subtitle_bits.append(f"v{version}")
    subtitle_bits.append(f"by {author}")
    if tags:
        subtitle_bits.append("  ".join(f"#{t}" for t in tags))
    ctx.header(name, subtitle="  ·  ".join(subtitle_bits))

    # Body area: wrap description, plus a status line + action hints.
    body_lines: list[str] = []
    wrapped_desc = ctx.wrap_text(
        desc, max_width_px=ctx.width - PAD * 2, size=BODY, monospace=False,
    )
    body_lines.extend(wrapped_desc)
    body_lines.append("")

    if installed:
        has_upd = _has_update(entry)
        registry_ver = entry.get("version", "")
        if has_upd:
            body_lines.append(f"Update available (v{registry_ver})")
        else:
            body_lines.append("Already installed")
        body_lines.append("")
        if state.confirm_uninstall:
            body_lines.append(f"Uninstall {name}? (y/n)")
    else:
        body_lines.append("Not installed")

    state.detail_scroll = ctx.scrollable_text(
        text_id="detail",
        lines=body_lines,
        scroll_offset=state.detail_scroll,
        line_height=BODY + 6,
        size=BODY,
    )

    # Status bar shortcuts depend on install state.
    if installed:
        has_upd = _has_update(entry)
        if state.confirm_uninstall:
            shortcuts = [("y", "confirm"), ("n", "cancel"), ("⌘W", "close")]
        elif has_upd:
            shortcuts = [
                ("u", "update"),
                ("x", "uninstall"),
                ("q/Backspace", "back"),
                ("⌘W", "close"),
            ]
        else:
            shortcuts = [
                ("x", "uninstall"),
                ("q/Backspace", "back"),
                ("⌘W", "close"),
            ]
    else:
        shortcuts = [
            ("i", "install"),
            ("q/Backspace", "back"),
            ("⌘W", "close"),
        ]
    ctx.status_bar(shortcuts)


def render_install(ctx, now: float):
    if not state.selected_entry:
        state.view = VIEW_BROWSE
        return

    name = state.selected_entry.get("name", "")
    ctx.header(f"Installing {name}")

    cx = ctx.width / 2
    cy = ctx.height / 2

    if state.install_done:
        ctx.text_center(cx, cy - BODY / 2, state.install_status,
                        size=BODY, color=THEME.green, bold=True)
        ctx.status_bar([("Backspace/q", "back"), ("⌘W", "close")])
    elif state.install_error:
        ctx.text_center(cx, cy - BODY / 2, state.install_status,
                        size=BODY, color=THEME.red)
        ctx.status_bar([("Backspace/q", "back"), ("⌘W", "close")])
    else:
        frame_idx = int(now * SPINNER_FPS) % len(SPINNER_FRAMES)
        spinner = SPINNER_FRAMES[frame_idx]
        ctx.text_center(cx, cy - TITLE - 4, spinner,
                        size=TITLE, color=THEME.accent, monospace=True)
        ctx.text_center(cx, cy, state.install_status or "Working...",
                        size=BODY, color=THEME.fg)
        ctx.status_bar([("⌘W", "close")])


# ---------------------------------------------------------------------------
# Top-level render
# ---------------------------------------------------------------------------

@app.on_render
def render(ctx):
    now = state.elapsed
    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

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
def on_key(key, _mods, emit):
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

        if key in ("j", "ArrowDown"):
            state.detail_scroll += 1
        elif key in ("k", "ArrowUp"):
            state.detail_scroll = max(0, state.detail_scroll - 1)
        elif key in ("Backspace", "q", "Escape"):
            state.view = VIEW_BROWSE
            state.confirm_uninstall = False
            state.detail_scroll = 0
        elif key == "i" and not is_installed(app_id):
            start_install(entry)
        elif key == "u" and is_installed(app_id) and _has_update(entry):
            # u = update (preserves user data files)
            start_update(entry, emit)
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
                state.detail_scroll = 0
        elif len(key) == 1:
            state.filter_text += key
            state.cursor = 0
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
    elif key == "Escape":
        state.filter_text = ""
        state.cursor = 0
    elif key == "Tab":
        state.tag_filter_idx = (state.tag_filter_idx + 1) % len(TAG_CYCLE)
        state.cursor = 0
    elif key == "Enter" and entries:
        state.selected_entry = entries[state.cursor]
        state.view = VIEW_DETAIL
        state.detail_scroll = 0
        state.confirm_uninstall = False
    elif key == "u" and entries:
        # Update the selected app in place, if it has an UPDATE badge
        sel = entries[state.cursor]
        if is_installed(sel.get("id", "")) and _has_update(sel):
            start_update(sel, emit)
        else:
            try:
                emit.info("u: selected app has no update available")
            except Exception:
                pass
    elif key == "U" and not state.update_all_running:
        start_update_all()


@app.on_scroll
def on_scroll(x, y, delta_x, delta_y, _emit):
    # Cursor-driven: scrollable_list auto-scrolls to keep selected visible,
    # so emulating a mouse wheel means nudging the cursor.
    if state.view == VIEW_BROWSE:
        entries = state.filtered_entries()
        if not entries:
            return
        if delta_y > 0:
            state.cursor = min(state.cursor + 1, len(entries) - 1)
        elif delta_y < 0:
            state.cursor = max(state.cursor - 1, 0)


app.run()

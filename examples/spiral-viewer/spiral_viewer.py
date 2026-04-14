#!/usr/bin/env python3
"""
spiral_viewer — render a target Plexi app at 8 sizes simultaneously on a
Fibonacci spiral. Developer tool for visually verifying app responsiveness
across breakpoints.

Usage (from within Plexi):
    spiral_viewer snake
    spiral_viewer /path/to/manifest.toml

Keybindings:
    r         re-spawn all instances (manual refresh)
    + / =     increase instance count
    - / _     decrease instance count
    q / Esc   quit

Architecture:
    On startup, argv[1] identifies a target Plexi app. Spiral viewer reads
    the target's manifest for min_width / min_height, then computes a
    logarithmic (Fibonacci-style) spiral of N positions. For each position,
    it spawns a subprocess of the target, sends init + render, captures the
    draw command stream, and re-emits those commands inside spiral viewer's
    own canvas — translated and scaled to the target position.

Stdlib only. No third-party deps.
"""
from __future__ import annotations

import json
import math
import os
import pathlib
import queue
import subprocess
import sys
import threading
import time
from typing import Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402

# ---------------------------------------------------------------------------
# Colors (Catppuccin Mocha-ish)
# ---------------------------------------------------------------------------

C_BG = "#11111b"
C_PANEL = "#1e1e2e"
C_BORDER = "#444455"
C_BORDER_ACTIVE = "#89b4fa"
C_LABEL = "#cdd6f4"
C_DIM = "#6c7086"
C_ERROR_RECT = "#3a1e1e"
C_ERROR_TEXT = "#f38ba8"
C_UNRENDERABLE = "#2a2a3a"
C_UNRENDERABLE_TEXT = "#6c7086"

# ---------------------------------------------------------------------------
# Spiral math
# ---------------------------------------------------------------------------

PHI = (1.0 + math.sqrt(5.0)) / 2.0  # 1.61803...


def spiral_positions(
    n: int,
    pane_w: float,
    pane_h: float,
    target_min_w: float = 200.0,
    target_min_h: float = 150.0,
) -> list[tuple[float, float, float, float]]:
    """
    Compute N (cx, cy, w, h) positions along a logarithmic spiral.

    Smallest instance at the center (spiral index 0), largest on the outside.
    Sizes range from (target_min_w*0.9, target_min_h*0.9) up to ~half of the
    spiral viewer's pane size — giving a broad coverage of breakpoints both
    below and above the target's stated min.

    Formula (per index i in [0, n)):
        frac       = i / (n - 1)              # 0..1
        size_w     = lerp(min_w*0.9, pane_w*0.45, frac)
        size_h     = lerp(min_h*0.9, pane_h*0.45, frac)
        r          = base_radius * PHI**(i/4)
        theta      = i * (2π / PHI²)

    Positions are clamped to stay inside the pane (leaving a margin for the
    label and border).
    """
    if n <= 0:
        return []

    cx0 = pane_w / 2.0
    cy0 = pane_h / 2.0
    # Keep the spiral compact enough that the biggest rect still fits
    # inside the pane with a margin.
    margin = 12.0
    max_w = max(target_min_w * 0.5, pane_w * 0.45)
    max_h = max(target_min_h * 0.5, pane_h * 0.45)
    base_radius = min(pane_w, pane_h) * 0.08

    positions: list[tuple[float, float, float, float]] = []
    for i in range(n):
        frac = 0.0 if n == 1 else i / (n - 1)
        w = target_min_w * 0.9 + (max_w - target_min_w * 0.9) * frac
        h = target_min_h * 0.9 + (max_h - target_min_h * 0.9) * frac

        r = base_radius * (PHI ** (i / 4.0))
        theta = i * (2.0 * math.pi / (PHI * PHI))
        cx = cx0 + r * math.cos(theta)
        cy = cy0 + r * math.sin(theta)

        # Clamp so the rect stays inside the pane.
        min_x = margin
        max_x = pane_w - w - margin
        min_y = margin + 18.0  # leave room for label
        max_y = pane_h - h - margin
        if max_x < min_x:
            max_x = min_x
        if max_y < min_y:
            max_y = min_y
        cx = max(min_x, min(max_x, cx - w / 2.0))
        cy = max(min_y, min(max_y, cy - h / 2.0))

        positions.append((cx, cy, w, h))
    return positions


# ---------------------------------------------------------------------------
# Vendored subprocess harness (minimal subset of plexi_test.AppTestHarness)
# ---------------------------------------------------------------------------


class _InstanceHarness:
    """
    Spawn a single target app subprocess. Drive it with init + render and
    capture draw commands. Guarantees cleanup on close().

    Not reused across refreshes — we spawn fresh each cycle to pick up file
    changes to the target source.
    """

    def __init__(self, entry_point: str, env_extra: dict):
        self.entry_point = os.path.abspath(entry_point)
        self._q: queue.Queue[dict] = queue.Queue()
        self._stderr: list[str] = []
        self._closed = False

        env = os.environ.copy()
        env["PYTHONUNBUFFERED"] = "1"
        env.update(env_extra)

        self._proc = subprocess.Popen(
            [sys.executable, self.entry_point],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=os.path.dirname(self.entry_point),
            env=env,
        )

        self._stdout_thread = threading.Thread(target=self._read_stdout, daemon=True)
        self._stdout_thread.start()
        self._stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self._stderr_thread.start()

    def _read_stdout(self):
        assert self._proc.stdout is not None
        for raw in self._proc.stdout:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue
            try:
                self._q.put(json.loads(line))
            except json.JSONDecodeError:
                pass

    def _read_stderr(self):
        assert self._proc.stderr is not None
        for raw in self._proc.stderr:
            line = raw.decode("utf-8", errors="replace").rstrip()
            self._stderr.append(line)

    def _send(self, event: dict):
        if self._proc.poll() is not None:
            raise RuntimeError(
                f"target exited early (code {self._proc.returncode}); "
                f"stderr: {self.stderr_text()}"
            )
        assert self._proc.stdin is not None
        data = (json.dumps(event) + "\n").encode("utf-8")
        try:
            self._proc.stdin.write(data)
            self._proc.stdin.flush()
        except (BrokenPipeError, OSError) as e:
            raise RuntimeError(f"write to target stdin failed: {e}") from e

    def send_init(self, width: int, height: int, launch_dir: str):
        self._send(
            {
                "type": "init",
                "width": width,
                "height": height,
                "launch_dir": launch_dir,
                "pixels_per_point": 1.0,
            }
        )

    def render_once(self, width: int, height: int, timeout: float = 2.0) -> list[dict]:
        self._send(
            {"type": "render", "width": width, "height": height, "delta_time": 0.0}
        )
        return self._read_until_frame_done(timeout=timeout)

    def _read_until_frame_done(self, timeout: float) -> list[dict]:
        cmds: list[dict] = []
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            try:
                obj = self._q.get(timeout=min(remaining, 0.1))
            except queue.Empty:
                continue
            if obj.get("type") == "frame_done":
                return cmds
            cmds.append(obj)
        raise TimeoutError(
            f"target render timed out after {timeout}s "
            f"(got {len(cmds)} commands so far)"
        )

    def stderr_text(self) -> str:
        return "\n".join(self._stderr) if self._stderr else ""

    def close(self):
        if self._closed:
            return
        self._closed = True
        try:
            if self._proc.poll() is None:
                try:
                    self._send({"type": "shutdown"})
                except Exception:
                    pass
        except Exception:
            pass
        try:
            self._proc.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            try:
                self._proc.terminate()
                self._proc.wait(timeout=0.5)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                try:
                    self._proc.wait(timeout=0.5)
                except Exception:
                    pass
        except Exception:
            pass

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass


# ---------------------------------------------------------------------------
# Manifest helpers
# ---------------------------------------------------------------------------


def _parse_flat_toml(text: str) -> dict:
    """
    Minimal TOML parser for our needs — sections + string/int/bool values.
    (Same approach as plexi_sdk.load_manifest.)
    """
    result: dict = {}
    current = result
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            keys = line[1:-1].split(".")
            current = result
            for k in keys:
                current = current.setdefault(k, {})
            continue
        if "=" in line:
            k, _, v = line.partition("=")
            k = k.strip()
            v = v.strip()
            # strip trailing comment
            if "#" in v and not (v.startswith('"') and v.count('"') >= 2):
                v = v.split("#", 1)[0].strip()
            if v.startswith('"') and v.endswith('"'):
                v = v[1:-1]
            elif v == "true":
                v = True  # type: ignore[assignment]
            elif v == "false":
                v = False  # type: ignore[assignment]
            else:
                try:
                    v = int(v)  # type: ignore[assignment]
                except ValueError:
                    pass
            current[k] = v
    return result


def _resolve_target(arg: Optional[str]) -> Optional[dict]:
    """
    Resolve argv[1] to a target descriptor:
        {
            "manifest_path": str,
            "entry_point": str,
            "app_id": str,
            "min_width": int,
            "min_height": int,
        }
    Returns None if nothing usable was provided.
    """
    if not arg:
        return None

    candidate_manifest: Optional[pathlib.Path] = None

    # 1. Direct file path?
    as_path = pathlib.Path(arg).expanduser()
    if as_path.is_file() and as_path.name == "manifest.toml":
        candidate_manifest = as_path
    elif as_path.is_dir():
        m = as_path / "manifest.toml"
        if m.is_file():
            candidate_manifest = m
    else:
        # 2. Lookup by id in installed apps dirs.
        search_dirs: list[pathlib.Path] = []
        env_dir = os.environ.get("PLEXI_APPS_DIR")
        if env_dir:
            search_dirs.append(pathlib.Path(env_dir))
        home = pathlib.Path.home()
        search_dirs.extend(
            [
                home / ".plexi-alpha" / "apps",
                home / ".plexi-beta" / "apps",
                home / ".plexi" / "apps",
            ]
        )
        # 3. Also check sibling examples/<arg>/manifest.toml
        here = pathlib.Path(os.path.dirname(os.path.abspath(__file__)))
        search_dirs.append(here.parent)  # examples/

        for d in search_dirs:
            m = d / arg / "manifest.toml"
            if m.is_file():
                candidate_manifest = m
                break

    if candidate_manifest is None:
        return None

    try:
        manifest = _parse_flat_toml(candidate_manifest.read_text())
    except OSError:
        return None
    app_section = manifest.get("app", {}) or {}
    entry = app_section.get("entry", "")
    if not entry:
        return None
    entry_path = candidate_manifest.parent / entry
    if not entry_path.is_file():
        return None
    layout = app_section.get("layout", {}) or {}
    return {
        "manifest_path": str(candidate_manifest),
        "entry_point": str(entry_path),
        "app_id": app_section.get("id", "unknown"),
        "min_width": int(layout.get("min_width", 400) or 400),
        "min_height": int(layout.get("min_height", 300) or 300),
    }


# ---------------------------------------------------------------------------
# Draw command translation
# ---------------------------------------------------------------------------

# Commands we know how to re-emit inside our own canvas.
TRANSLATABLE = {"rect", "text", "line", "image"}


def _translate_commands(
    commands: list[dict],
    origin_x: float,
    origin_y: float,
    target_w: float,
    target_h: float,
    display_w: float,
    display_h: float,
    ctx,
) -> int:
    """
    Walk the target app's draw commands and re-emit them on ctx, mapping from
    the target's local coordinate space (target_w × target_h) to the display
    region (origin_x, origin_y, display_w, display_h).

    Returns the number of commands skipped (unrenderable).
    """
    if target_w <= 0 or target_h <= 0:
        return len(commands)
    sx = display_w / float(target_w)
    sy = display_h / float(target_h)
    skipped = 0

    for cmd in commands:
        t = cmd.get("type")
        if t == "rect":
            x = origin_x + float(cmd.get("x", 0)) * sx
            y = origin_y + float(cmd.get("y", 0)) * sy
            w = float(cmd.get("w", 0)) * sx
            h = float(cmd.get("h", 0)) * sy
            ctx.rect(
                x,
                y,
                w,
                h,
                fill=cmd.get("fill", "#000000"),
                radius=float(cmd.get("radius", 0)) * min(sx, sy),
            )
        elif t == "text":
            x = origin_x + float(cmd.get("x", 0)) * sx
            y = origin_y + float(cmd.get("y", 0)) * sy
            size = max(6.0, float(cmd.get("size", 12)) * min(sx, sy))
            ctx.text(
                x,
                y,
                str(cmd.get("text", "")),
                size=size,
                color=cmd.get("color", "#cdd6f4"),
                monospace=bool(cmd.get("monospace", False)),
                bold=bool(cmd.get("bold", False)),
            )
        elif t == "line":
            x1 = origin_x + float(cmd.get("x1", 0)) * sx
            y1 = origin_y + float(cmd.get("y1", 0)) * sy
            x2 = origin_x + float(cmd.get("x2", 0)) * sx
            y2 = origin_y + float(cmd.get("y2", 0)) * sy
            ctx.line(
                x1,
                y1,
                x2,
                y2,
                color=cmd.get("color", "#cdd6f4"),
                width=max(0.5, float(cmd.get("width", 1.0)) * min(sx, sy)),
            )
        elif t == "image":
            x = origin_x + float(cmd.get("x", 0)) * sx
            y = origin_y + float(cmd.get("y", 0)) * sy
            w = float(cmd.get("w", 0)) * sx
            h = float(cmd.get("h", 0)) * sy
            path = str(cmd.get("path", ""))
            if path:
                ctx.image(path, x, y, w, h, fit=str(cmd.get("fit", "contain")))
        elif t in ("frame_done",):
            continue
        else:
            # Unrenderable (list, file_grid, video_thumbnail, set_cursor, etc.)
            skipped += 1

    return skipped


# ---------------------------------------------------------------------------
# Instance state
# ---------------------------------------------------------------------------


class Instance:
    __slots__ = (
        "cx",
        "cy",
        "w",
        "h",
        "draw_commands",
        "skipped",
        "error",
        "last_render_time",
    )

    def __init__(self, cx: float, cy: float, w: float, h: float):
        self.cx = cx
        self.cy = cy
        self.w = w
        self.h = h
        self.draw_commands: list[dict] = []
        self.skipped: int = 0
        self.error: Optional[str] = None
        self.last_render_time: float = 0.0


# ---------------------------------------------------------------------------
# Viewer
# ---------------------------------------------------------------------------


REFRESH_INTERVAL_SEC = 5.0
INSTANCE_COUNTS = [4, 6, 8, 12, 16]


class SpiralViewer:
    def __init__(self, target: Optional[dict]):
        self.target = target
        self.instance_count: int = 8
        self.instances: list[Instance] = []
        self.last_refresh_time: float = 0.0
        self.last_pane_size: tuple[float, float] = (0.0, 0.0)
        self.status: str = "ready" if target else "no target"
        self.selected_label: Optional[str] = None

    def _spawn_and_capture(
        self, pane_w: float, pane_h: float, emitter=None
    ) -> None:
        """Fresh spawn cycle. Kills any previous processes implicitly by not
        holding a reference past this function."""
        self.instances.clear()
        self.last_refresh_time = time.monotonic()
        self.last_pane_size = (pane_w, pane_h)
        if self.target is None:
            return

        positions = spiral_positions(
            self.instance_count,
            pane_w,
            pane_h,
            target_min_w=float(self.target["min_width"]),
            target_min_h=float(self.target["min_height"]),
        )

        env_extra = {
            "PLEXI_APP_ID": self.target["app_id"],
            "PLEXI_APPS_DIR": os.environ.get(
                "PLEXI_APPS_DIR",
                str(pathlib.Path.home() / ".plexi-alpha" / "apps"),
            ),
        }

        for (cx, cy, w, h) in positions:
            inst = Instance(cx, cy, w, h)
            harness: Optional[_InstanceHarness] = None
            try:
                harness = _InstanceHarness(self.target["entry_point"], env_extra)
                launch_dir = os.path.dirname(self.target["entry_point"])
                iw = max(40, int(round(w)))
                ih = max(30, int(round(h)))
                harness.send_init(iw, ih, launch_dir)
                cmds = harness.render_once(iw, ih, timeout=2.0)
                inst.draw_commands = cmds
                inst.last_render_time = time.monotonic()
            except (TimeoutError, RuntimeError, OSError) as e:
                inst.error = str(e)[:200]
                if emitter is not None:
                    try:
                        emitter.warn(f"spiral-viewer instance failed: {inst.error}")
                    except Exception:
                        pass
            finally:
                # Always clean up the subprocess — no orphans.
                if harness is not None:
                    harness.close()
            self.instances.append(inst)
        self.status = f"{len(self.instances)} instances"

    def maybe_refresh(self, pane_w: float, pane_h: float, emitter=None):
        now = time.monotonic()
        need_resize = (pane_w, pane_h) != self.last_pane_size
        if need_resize or (now - self.last_refresh_time) >= REFRESH_INTERVAL_SEC:
            self._spawn_and_capture(pane_w, pane_h, emitter=emitter)

    def force_refresh(self, pane_w: float, pane_h: float, emitter=None):
        self._spawn_and_capture(pane_w, pane_h, emitter=emitter)

    def change_instance_count(self, delta: int):
        try:
            idx = INSTANCE_COUNTS.index(self.instance_count)
        except ValueError:
            idx = 2  # default 8
        idx = max(0, min(len(INSTANCE_COUNTS) - 1, idx + delta))
        self.instance_count = INSTANCE_COUNTS[idx]
        # Force a refresh on next render
        self.last_pane_size = (-1.0, -1.0)

    def hit_test(self, x: float, y: float) -> Optional[Instance]:
        for inst in self.instances:
            if (
                inst.cx <= x <= inst.cx + inst.w
                and inst.cy <= y <= inst.cy + inst.h
            ):
                return inst
        return None


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


def render_frame(viewer: SpiralViewer, ctx, emitter) -> None:
    pane_w = ctx.width
    pane_h = ctx.height

    # Background
    ctx.rect(0, 0, pane_w, pane_h, fill=C_BG)

    # No target: instructional help
    if viewer.target is None:
        ctx.text(
            pane_w / 2 - 260,
            pane_h / 2 - 30,
            "Spiral Viewer",
            size=22,
            color=C_LABEL,
            bold=True,
        )
        ctx.text(
            pane_w / 2 - 260,
            pane_h / 2 + 5,
            "Pass an app path or id as argv[1].",
            size=14,
            color=C_LABEL,
        )
        ctx.text(
            pane_w / 2 - 260,
            pane_h / 2 + 28,
            "Example: spiral_viewer snake",
            size=13,
            color=C_DIM,
            monospace=True,
        )
        ctx.text(
            pane_w / 2 - 260,
            pane_h / 2 + 48,
            "Keys:  r refresh   +/- count   q quit",
            size=12,
            color=C_DIM,
        )
        return

    # Ensure we have instances for this pane size.
    viewer.maybe_refresh(pane_w, pane_h, emitter=emitter)

    # Header
    ctx.text(
        12,
        10,
        f"Spiral Viewer — {viewer.target['app_id']} "
        f"(min {viewer.target['min_width']}x{viewer.target['min_height']})",
        size=13,
        color=C_LABEL,
        bold=True,
    )
    ctx.text(
        pane_w - 220,
        10,
        f"instances: {viewer.instance_count}   r=refresh   +/- count",
        size=11,
        color=C_DIM,
    )
    if viewer.selected_label:
        ctx.text(
            12,
            pane_h - 18,
            f"selected: {viewer.selected_label}",
            size=11,
            color=C_BORDER_ACTIVE,
        )

    # Each instance
    for inst in viewer.instances:
        # panel bg
        ctx.rect(
            inst.cx, inst.cy, inst.w, inst.h, fill=C_PANEL, radius=2.0
        )
        # border (4 thin lines)
        x1, y1 = inst.cx, inst.cy
        x2, y2 = inst.cx + inst.w, inst.cy + inst.h
        ctx.line(x1, y1, x2, y1, color=C_BORDER, width=1.0)
        ctx.line(x2, y1, x2, y2, color=C_BORDER, width=1.0)
        ctx.line(x2, y2, x1, y2, color=C_BORDER, width=1.0)
        ctx.line(x1, y2, x1, y1, color=C_BORDER, width=1.0)

        # Label above
        label = f"{int(round(inst.w))}x{int(round(inst.h))}"
        ctx.text(
            inst.cx,
            max(2.0, inst.cy - 14),
            label,
            size=10,
            color=C_LABEL,
            monospace=True,
        )

        if inst.error:
            ctx.rect(
                inst.cx, inst.cy, inst.w, inst.h, fill=C_ERROR_RECT, radius=2.0
            )
            ctx.text(
                inst.cx + 4,
                inst.cy + 4,
                "render failed",
                size=10,
                color=C_ERROR_TEXT,
                bold=True,
            )
            ctx.text(
                inst.cx + 4,
                inst.cy + 18,
                inst.error[:60],
                size=9,
                color=C_ERROR_TEXT,
            )
            continue

        # Translate and re-emit target commands.
        skipped = _translate_commands(
            inst.draw_commands,
            origin_x=inst.cx,
            origin_y=inst.cy,
            target_w=inst.w,
            target_h=inst.h,
            display_w=inst.w,
            display_h=inst.h,
            ctx=ctx,
        )
        inst.skipped = skipped
        if skipped > 0:
            ctx.text(
                inst.cx + 4,
                inst.cy + inst.h - 12,
                f"{skipped} unrenderable",
                size=9,
                color=C_UNRENDERABLE_TEXT,
            )


# ---------------------------------------------------------------------------
# App wiring
# ---------------------------------------------------------------------------


def _argv_target() -> Optional[str]:
    # argv[1] if provided; otherwise env PLEXI_SPIRAL_TARGET (useful for tests).
    if len(sys.argv) > 1 and sys.argv[1]:
        return sys.argv[1]
    return os.environ.get("PLEXI_SPIRAL_TARGET") or None


def build_app() -> tuple[App, SpiralViewer]:
    target = _resolve_target(_argv_target())
    viewer = SpiralViewer(target)
    app = App(app_id="spiral-viewer")

    @app.on_render
    def _render(ctx):
        render_frame(viewer, ctx, app._emitter)

    @app.on_key
    def _key(key, mods, emit):
        k = (key or "").lower()
        if k in ("r",):
            pane_w, pane_h = viewer.last_pane_size
            if pane_w > 0 and pane_h > 0:
                viewer.force_refresh(pane_w, pane_h, emitter=emit)
            emit.info("spiral-viewer: manual refresh")
        elif k in ("+", "="):
            viewer.change_instance_count(+1)
            emit.info(f"spiral-viewer: count={viewer.instance_count}")
        elif k in ("-", "_"):
            viewer.change_instance_count(-1)
            emit.info(f"spiral-viewer: count={viewer.instance_count}")
        elif k in ("q", "escape", "esc"):
            # Let Plexi handle quit — spiral viewer is just a pane, not the
            # window. We just log and do nothing destructive.
            emit.info("spiral-viewer: quit requested")

    @app.on_click
    def _click(x, y, button, emit):
        inst = viewer.hit_test(x, y)
        if inst is not None:
            label = f"{int(round(inst.w))}x{int(round(inst.h))}"
            viewer.selected_label = label
            emit.info(f"spiral-viewer: clicked instance {label}")
            emit.notification(
                title="Spiral Viewer",
                body=f"Instance size: {label}",
                priority=0,
            )

    return app, viewer


def main():
    app, _viewer = build_app()
    app.run()


if __name__ == "__main__":
    main()

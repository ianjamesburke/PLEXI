"""Headless snapshot testing for Plexi widgets.

Spawns the plexi binary with `--render`, feeds it DrawCommand JSON,
reads back PNG bytes. Provides pixel-level assertions and snapshot file helpers.
"""

import json
import os
import queue
import struct
import subprocess
import sys
import threading
import time
import zlib
from pathlib import Path

DEFAULT_BG = "#1e1e2e"

# ---------------------------------------------------------------------------
# Binary location
# ---------------------------------------------------------------------------

def _find_renderer() -> str:
    """Locate the plexi binary to invoke with `--render`.

    Checks in order:
      1. PLEXI_RENDER_BIN env var (path to a plexi binary)
      2. <repo_root>/target/release/plexi
      3. <repo_root>/target/debug/plexi

    Raises RuntimeError with build instructions if not found.
    """
    env_override = os.environ.get("PLEXI_RENDER_BIN")
    if env_override:
        if os.path.isfile(env_override) and os.access(env_override, os.X_OK):
            return env_override
        raise RuntimeError(
            f"PLEXI_RENDER_BIN is set to {env_override!r} but the file is not "
            "executable or does not exist."
        )

    # Resolve repo root relative to this file: sdk/python/plexi_sdk/testing.py
    # → walk up three parents to reach the repo root.
    repo_root = Path(__file__).resolve().parent.parent.parent.parent
    release = repo_root / "target" / "release" / "plexi"
    debug = repo_root / "target" / "debug" / "plexi"

    for candidate in (release, debug):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)

    raise RuntimeError(
        "plexi binary not found. Build it with:\n"
        "  cargo build --release\n"
        f"Expected at: {release}"
    )


# ---------------------------------------------------------------------------
# Core render call
# ---------------------------------------------------------------------------

def render_draw_commands(
    commands: list[dict],
    width: int,
    height: int,
    background: str = DEFAULT_BG,
) -> bytes:
    """Feed draw commands to the headless renderer, return PNG bytes.

    Args:
        commands: list of PGAP draw command dicts (rect, text, line, …).
        width:    viewport width in pixels.
        height:   viewport height in pixels.
        background: CSS hex colour string for the background fill.

    Returns:
        Raw PNG bytes.

    Raises:
        RuntimeError if the binary is not found or exits nonzero.
    """
    renderer = _find_renderer()
    payload = json.dumps({
        "viewport": {"width": width, "height": height, "background": background},
        "draw_commands": commands,
    })
    try:
        result = subprocess.run(
            [renderer, "--render"],
            input=payload.encode(),
            capture_output=True,
            check=False,
        )
    except OSError as e:
        raise RuntimeError(f"Failed to spawn plexi --render ({renderer}): {e}") from e

    if result.returncode != 0:
        stderr = result.stderr.decode(errors="replace").strip()
        raise RuntimeError(
            f"plexi --render exited {result.returncode}: {stderr}"
        )

    return result.stdout


# ---------------------------------------------------------------------------
# PNG decoder (stdlib only — no PIL/Pillow)
# ---------------------------------------------------------------------------

def decode_png(png_bytes: bytes) -> tuple[int, int, bytes]:
    """Decode an RGBA 8-bit PNG to (width, height, raw_rgba_bytes).

    Pure stdlib implementation using zlib + struct. Only handles RGBA 8-bit
    PNGs — which is exactly what tiny-skia produces.

    Raises:
        ValueError on an unrecognised PNG or unsupported colour type/bit depth.
    """
    if not png_bytes[:8] == b"\x89PNG\r\n\x1a\n":
        raise ValueError("Not a PNG file (bad signature)")

    # Parse chunks.
    pos = 8
    width = height = 0
    raw_idat = b""

    while pos < len(png_bytes):
        length = struct.unpack_from(">I", png_bytes, pos)[0]
        chunk_type = png_bytes[pos + 4: pos + 8]
        data = png_bytes[pos + 8: pos + 8 + length]
        pos += 12 + length  # length + type(4) + data + crc(4)

        if chunk_type == b"IHDR":
            width, height = struct.unpack_from(">II", data)
            bit_depth = data[8]
            colour_type = data[9]
            if bit_depth != 8 or colour_type != 6:
                raise ValueError(
                    f"Unsupported PNG colour type={colour_type} bit_depth={bit_depth}. "
                    "Only RGBA 8-bit (colour_type=6) is supported."
                )
        elif chunk_type == b"IDAT":
            raw_idat += data
        elif chunk_type == b"IEND":
            break

    # Decompress all IDAT chunks together.
    decompressed = zlib.decompress(raw_idat)

    # Reconstruct pixel data by reversing the PNG filter on each scanline.
    bytes_per_pixel = 4  # RGBA
    stride = width * bytes_per_pixel
    raw_rgba = bytearray(stride * height)

    for row in range(height):
        filter_byte = decompressed[row * (stride + 1)]
        line_start = row * (stride + 1) + 1
        line = decompressed[line_start: line_start + stride]

        if filter_byte == 0:  # None
            raw_rgba[row * stride: (row + 1) * stride] = line
        elif filter_byte == 1:  # Sub
            out = bytearray(line)
            for i in range(bytes_per_pixel, stride):
                out[i] = (out[i] + out[i - bytes_per_pixel]) & 0xFF
            raw_rgba[row * stride: (row + 1) * stride] = out
        elif filter_byte == 2:  # Up
            out = bytearray(line)
            if row > 0:
                prev = raw_rgba[(row - 1) * stride: row * stride]
                for i in range(stride):
                    out[i] = (out[i] + prev[i]) & 0xFF
            raw_rgba[row * stride: (row + 1) * stride] = out
        elif filter_byte == 3:  # Average
            out = bytearray(line)
            prev = raw_rgba[(row - 1) * stride: row * stride] if row > 0 else bytes(stride)
            for i in range(stride):
                a = out[i - bytes_per_pixel] if i >= bytes_per_pixel else 0
                b = prev[i]
                out[i] = (out[i] + (a + b) // 2) & 0xFF
            raw_rgba[row * stride: (row + 1) * stride] = out
        elif filter_byte == 4:  # Paeth
            out = bytearray(line)
            prev = raw_rgba[(row - 1) * stride: row * stride] if row > 0 else bytes(stride)
            for i in range(stride):
                a = out[i - bytes_per_pixel] if i >= bytes_per_pixel else 0
                b = prev[i]
                c = prev[i - bytes_per_pixel] if i >= bytes_per_pixel else 0
                p = a + b - c
                pa = abs(p - a)
                pb = abs(p - b)
                pc = abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                out[i] = (out[i] + pr) & 0xFF
            raw_rgba[row * stride: (row + 1) * stride] = out
        else:
            raise ValueError(f"Unknown PNG filter byte {filter_byte} at row {row}")

    return width, height, bytes(raw_rgba)


# ---------------------------------------------------------------------------
# Pixel helpers
# ---------------------------------------------------------------------------

def pixel_at(png_bytes: bytes, x: int, y: int) -> tuple[int, int, int, int]:
    """Return (r, g, b, a) of the pixel at (x, y)."""
    width, height, raw = decode_png(png_bytes)
    if x < 0 or x >= width or y < 0 or y >= height:
        raise ValueError(f"Pixel ({x}, {y}) out of bounds for {width}×{height} image")
    idx = (y * width + x) * 4
    return raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]


def hex_to_rgba(hex_color: str) -> tuple[int, int, int, int]:
    """Convert a CSS hex color string to (r, g, b, a).

    Supports #RGB, #RRGGBB, and #RRGGBBAA.
    Alpha defaults to 255 when not specified.
    """
    s = hex_color.lstrip("#")
    if len(s) == 3:
        r = int(s[0] * 2, 16)
        g = int(s[1] * 2, 16)
        b = int(s[2] * 2, 16)
        return r, g, b, 255
    elif len(s) == 6:
        r = int(s[0:2], 16)
        g = int(s[2:4], 16)
        b = int(s[4:6], 16)
        return r, g, b, 255
    elif len(s) == 8:
        r = int(s[0:2], 16)
        g = int(s[2:4], 16)
        b = int(s[4:6], 16)
        a = int(s[6:8], 16)
        return r, g, b, a
    else:
        raise ValueError(f"Unrecognised hex color: {hex_color!r}")


def assert_pixel(
    png_bytes: bytes,
    x: int,
    y: int,
    expected: str | tuple[int, int, int, int],
    tolerance: int = 4,
) -> None:
    """Assert the pixel at (x, y) matches the expected color within per-channel tolerance.

    Args:
        png_bytes: raw PNG bytes from render_draw_commands.
        x, y:     pixel coordinates.
        expected: CSS hex string (e.g. "#ff0000") or (r, g, b, a) tuple.
        tolerance: per-channel absolute tolerance (default 4).

    Raises:
        AssertionError with a diagnostic message on mismatch.
    """
    if isinstance(expected, str):
        exp = hex_to_rgba(expected)
    else:
        exp = expected

    got = pixel_at(png_bytes, x, y)
    channels = ("r", "g", "b", "a")
    mismatches = []
    for ch, g, e in zip(channels, got, exp):
        if abs(g - e) > tolerance:
            mismatches.append(f"  {ch}: got {g}, expected {e} (diff {abs(g - e)} > tolerance {tolerance})")

    if mismatches:
        exp_hex = "#{:02x}{:02x}{:02x}{:02x}".format(*exp)
        got_hex = "#{:02x}{:02x}{:02x}{:02x}".format(*got)
        raise AssertionError(
            f"Pixel ({x}, {y}) mismatch:\n"
            f"  expected {exp_hex}, got {got_hex}\n"
            + "\n".join(mismatches)
        )


# ---------------------------------------------------------------------------
# Snapshot file helper
# ---------------------------------------------------------------------------

def save_snapshot(png_bytes: bytes, path: str | Path) -> None:
    """Write PNG bytes to path. Creates parent directories as needed."""
    p = Path(path)
    try:
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(png_bytes)
    except OSError as e:
        raise RuntimeError(f"Failed to save snapshot to {p}: {e}") from e


# ---------------------------------------------------------------------------
# AppHarness — headless Python app subprocess runner (issue #865)
# ---------------------------------------------------------------------------


class AppHarness:
    """Headless runner for Plexi Python apps.

    Spawns the app script as a subprocess, communicates over stdin/stdout
    using the PGAP v3 protocol. No display or running Plexi host required.

    Usage::

        with AppHarness("my_app.py", width=400, height=300) as h:
            cmds = h.run(1)                  # step one render frame
            h.key("enter")                   # inject a key event
            cmds = h.run(1)                  # render again to see effects
            h.assert_pixel(10, 10, "#1e1e2e")  # pixel assertion (needs plexi binary)
    """

    def __init__(self, app_path: "str | Path", width: int = 800, height: int = 600,
                 timeout: float = 5.0) -> None:
        self._width = width
        self._height = height
        self._timeout = timeout
        self._draw_commands: "list[dict]" = []
        self._frame_id = 0
        self._stdout_q: "queue.Queue[str | None]" = queue.Queue()

        import os
        import subprocess as _subprocess
        env = dict(os.environ)
        env["PYTHONPATH"] = os.pathsep.join(sys.path)

        self._proc = _subprocess.Popen(
            [sys.executable, str(app_path)],
            stdin=_subprocess.PIPE,
            stdout=_subprocess.PIPE,
            stderr=_subprocess.PIPE,
            text=True,
            env=env,
        )

        self._reader = threading.Thread(target=self._read_stdout, daemon=True)
        self._reader.start()

        # Handshake
        self._send({"type": "init", "protocol": "pgap/3", "app_id": "test-harness",
                    "workspace_root": "/tmp", "capabilities": [], "feature_flags": []})
        ev = self._wait_for(lambda e: e.get("type") == "ready")
        if ev is None:
            stderr = self._proc.stderr.read() if self._proc.stderr else ""
            raise RuntimeError(
                f"AppHarness: app did not send 'ready' within {timeout}s. "
                f"stderr: {stderr[:500]!r}"
            )

    def _read_stdout(self) -> None:
        assert self._proc.stdout is not None
        for line in self._proc.stdout:
            line = line.strip()
            if line:
                self._stdout_q.put(line)
        self._stdout_q.put(None)  # EOF sentinel

    def _next_event(self, timeout: float) -> "dict | None":
        try:
            line = self._stdout_q.get(timeout=timeout)
        except queue.Empty:
            return None
        if line is None:
            return None
        try:
            return json.loads(line)
        except json.JSONDecodeError:
            return None

    def _wait_for(self, predicate, timeout=None) -> "dict | None":
        deadline = time.monotonic() + (timeout if timeout is not None else self._timeout)
        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            ev = self._next_event(timeout=max(0.05, remaining))
            if ev is not None and predicate(ev):
                return ev
        return None

    def _send(self, event: dict) -> None:
        assert self._proc.stdin is not None
        self._proc.stdin.write(json.dumps(event) + "\n")
        self._proc.stdin.flush()

    def run(self, n_frames: int = 1) -> "list[dict]":
        """Step N render frames. Returns draw commands from the last frame."""
        cmds: "list[dict]" = []
        for _ in range(n_frames):
            self._frame_id += 1
            self._send({
                "type": "render",
                "frame_id": self._frame_id,
                "rect": {"x": 0.0, "y": 0.0, "w": float(self._width), "h": float(self._height)},
            })
            cmds = []
            deadline = time.monotonic() + self._timeout
            while time.monotonic() < deadline:
                remaining = deadline - time.monotonic()
                ev = self._next_event(timeout=max(0.05, remaining))
                if ev is None:
                    continue
                if ev.get("type") == "frame_done":
                    break
                cmds.append(ev)
        self._draw_commands = cmds
        return cmds

    def key(self, key: str, modifiers: "dict | None" = None) -> None:
        """Inject a synthetic key event."""
        self._send({"type": "key", "key": key, "modifiers": modifiers or {}})

    def text_submit(self, id: str, value: str) -> None:
        """Inject a synthetic text_submitted event (user pressed Enter in a TextInput)."""
        self._send({"type": "text_submitted", "id": id, "value": value})

    def screenshot(self) -> bytes:
        """Render current draw commands to PNG. Requires the plexi binary."""
        return render_draw_commands(self._draw_commands, self._width, self._height)

    def assert_pixel(self, x: int, y: int,
                     expected: "str | tuple[int, int, int, int]",
                     tolerance: int = 4) -> None:
        """Assert pixel color at (x, y) against the last rendered frame."""
        assert_pixel(self.screenshot(), x, y, expected, tolerance)

    def save_snapshot(self, path: "str | Path") -> None:
        """Save the last rendered frame as a PNG file."""
        save_snapshot(self.screenshot(), path)

    def close(self) -> None:
        """Shut down the app subprocess cleanly."""
        import subprocess as _subprocess
        try:
            if self._proc.stdin:
                self._proc.stdin.close()
        except OSError:
            pass
        try:
            self._proc.wait(timeout=5.0)
        except _subprocess.TimeoutExpired:
            self._proc.kill()

    def __enter__(self) -> "AppHarness":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

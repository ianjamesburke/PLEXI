"""
plexi_test.py — Python test harness for PGAP v3 apps.

Drop this alongside plexi_sdk.py in any app directory, or import from
the examples/ root. Use with pytest.

Usage:
    from plexi_test import Harness

    def test_my_app():
        with Harness.for_app("myapp.py") as h:
            h.init()
            cmds = h.render_frame(800, 600)
            texts = [c["text"] for c in cmds if c.get("type") == "text"]
            assert "Hello" in texts
"""
from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any


class Harness:
    """
    Subprocess driver for a PGAP v3 app.

    Spawns the app, owns stdin/stdout routing, and intercepts http_request
    commands from the app's background threads in real time (no pre-drain
    race — handled in the reader thread).
    """

    def __init__(self) -> None:
        self._proc: subprocess.Popen | None = None
        self._stdin_lock = threading.Lock()
        self._rx: queue.Queue[dict] = queue.Queue()
        self._http_mocks: dict[str, str] = {}
        self._frame_counter = 0

    # ── Factory ────────────────────────────────────────────────────────────────

    @classmethod
    def for_app(cls, script: str | Path, args: list[str] | None = None,
                cwd: str | Path | None = None) -> "Harness":
        """Spawn a PGAP app from a script path. Prefer `with Harness.for_app(...) as h:`."""
        h = cls()
        script = Path(script)
        if cwd is None:
            cwd = script.parent
        env = os.environ.copy()
        # Point media subsystem at mock devices so tests stay deterministic.
        tmp = Path("/tmp")
        env.setdefault("PLEXI_AUDIO", f"mock://{tmp}/plexi-test-in.wav,{tmp}/plexi-test-out.wav")
        env.setdefault("PLEXI_VIDEO", "mock://fixture.mp4?duration=1000&w=32&h=32")

        h._proc = subprocess.Popen(
            [sys.executable, str(script)] + (args or []),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=str(cwd),
            env=env,
        )
        threading.Thread(target=h._read_loop, daemon=True).start()
        threading.Thread(target=h._err_loop, daemon=True).start()
        return h

    # ── Context manager ────────────────────────────────────────────────────────

    def __enter__(self) -> "Harness":
        return self

    def __exit__(self, *_: Any) -> None:
        self.shutdown()

    # ── Low-level I/O ──────────────────────────────────────────────────────────

    def _send(self, obj: dict) -> None:
        with self._stdin_lock:
            assert self._proc and self._proc.stdin
            line = json.dumps(obj) + "\n"
            self._proc.stdin.write(line.encode())
            self._proc.stdin.flush()

    def _read_loop(self) -> None:
        assert self._proc and self._proc.stdout
        for raw in self._proc.stdout:
            line = raw.decode().strip()
            if not line:
                continue
            try:
                v = json.loads(line)
            except json.JSONDecodeError:
                continue
            # Intercept http_request immediately in the reader thread — no race.
            if v.get("type") == "http_request":
                self._reply_http(v)
            else:
                self._rx.put(v)

    def _err_loop(self) -> None:
        assert self._proc and self._proc.stderr
        for raw in self._proc.stderr:
            print(f"APP STDERR: {raw.decode().rstrip()}", file=sys.stderr)

    def _reply_http(self, v: dict) -> None:
        req_id = v.get("request_id", "")
        url = v.get("url", "")
        if url in self._http_mocks:
            self._send({"type": "http_response", "request_id": req_id,
                        "status": 200, "body": self._http_mocks[url]})
        else:
            self._send({"type": "http_response", "request_id": req_id,
                        "status": 404, "body": "",
                        "error": f"no mock registered for {url!r}"})

    def _recv(self, timeout: float = 5.0) -> dict | None:
        try:
            return self._rx.get(timeout=timeout)
        except queue.Empty:
            return None

    # ── PGAP protocol ──────────────────────────────────────────────────────────

    def init(self, app_id: str = "test", workspace_root: str = "/tmp") -> dict:
        """Send Init, wait for Ready. Returns the Ready payload."""
        self._send({
            "type": "init",
            "protocol": "pgap/3",
            "app_id": app_id,
            "workspace_root": workspace_root,
            "capabilities": [],
            "feature_flags": ["media_v1", "pane_groups_v1"],
        })
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            v = self._recv(timeout=5)
            if v and v.get("type") == "ready":
                return v
        raise TimeoutError("did not receive Ready within 10s")

    def render_frame(self, w: float, h: float) -> list[dict]:
        """Send Render, collect DrawCommands up to FrameDone. Returns draw commands."""
        self._frame_counter += 1
        frame_id = self._frame_counter
        self._send({"type": "render", "frame_id": frame_id,
                    "rect": {"x": 0.0, "y": 0.0, "w": w, "h": h}})
        cmds: list[dict] = []
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            v = self._recv(timeout=5)
            if v is None:
                break
            cmds.append(v)
            if v.get("type") == "frame_done":
                assert v.get("frame_id") == frame_id, \
                    f"frame_done id mismatch: got {v.get('frame_id')}, want {frame_id}"
                return cmds
        raise TimeoutError(f"did not receive frame_done({frame_id}); got {len(cmds)} cmds")

    def inject_state(self, payload: dict) -> None:
        """Send InjectState. The app's on_inject handler runs synchronously."""
        self._send({"type": "inject_state", "payload": payload})
        time.sleep(0.03)  # let the PGAP loop process before next call

    def send_key(self, key: str, shift: bool = False, ctrl: bool = False,
                 alt: bool = False, cmd: bool = False) -> None:
        self._send({"type": "key", "key": key,
                    "modifiers": {"shift": shift, "ctrl": ctrl, "alt": alt, "cmd": cmd}})

    def send_path_changed(self, cwd: str) -> None:
        self._send({"type": "path_changed", "cwd": cwd})

    def mock_http(self, url: str, body: str) -> None:
        """Register a mock HTTP response. Exact URL match."""
        self._http_mocks[url] = body

    def shutdown(self) -> None:
        if self._proc is None:
            return
        try:
            self._send({"type": "shutdown"})
            time.sleep(0.1)
        except Exception:
            pass
        try:
            self._proc.kill()
            self._proc.wait(timeout=2)
        except Exception:
            pass
        self._proc = None

    # ── Assertion helpers ──────────────────────────────────────────────────────

    def texts(self, cmds: list[dict]) -> list[str]:
        """Extract all text values from a frame's draw commands."""
        return [c["text"] for c in cmds if c.get("type") == "text"]

    def has_text(self, cmds: list[dict], substring: str) -> bool:
        return any(substring in t for t in self.texts(cmds))

    def rect_count(self, cmds: list[dict]) -> int:
        return sum(1 for c in cmds if c.get("type") == "rect")

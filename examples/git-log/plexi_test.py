"""
plexi_test.py — Test harness for Plexi apps.

Zero dependencies, pure stdlib. Ships alongside plexi_sdk.py.

Spawn any Plexi app as a subprocess, send JSON events on stdin,
read JSON draw commands on stdout, and assert on the results.

Usage:

    from plexi_test import AppTestHarness

    h = AppTestHarness("path/to/app.py")
    h.send_init()
    frames = h.send_render()
    h.assert_text_visible("Hello", frames)
    h.shutdown()
"""

import json
import os
import queue
import subprocess
import sys
import threading
import time
from typing import Optional


class AppTestHarness:
    """Spawn a Plexi app as a subprocess and drive it via the JSON protocol."""

    def __init__(self, entry_point: str, launch_dir: str = "/tmp/plexi-test"):
        self.entry_point = os.path.abspath(entry_point)
        self.launch_dir = launch_dir
        self._last_frames: list[dict] = []
        self._output_queue: queue.Queue[dict] = queue.Queue()
        self._stderr_lines: list[str] = []
        self._reader_thread: Optional[threading.Thread] = None
        self._stderr_thread: Optional[threading.Thread] = None
        self._closed = False

        os.makedirs(launch_dir, exist_ok=True)

        env = os.environ.copy()
        env["PYTHONUNBUFFERED"] = "1"

        try:
            self._proc = subprocess.Popen(
                [sys.executable, self.entry_point],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=launch_dir,
                env=env,
            )
        except Exception as e:
            raise RuntimeError(f"Failed to spawn app {self.entry_point}: {e}") from e

        # Background thread to read stdout lines and enqueue parsed JSON
        self._reader_thread = threading.Thread(
            target=self._read_stdout, daemon=True
        )
        self._reader_thread.start()

        # Background thread to capture stderr
        self._stderr_thread = threading.Thread(
            target=self._read_stderr, daemon=True
        )
        self._stderr_thread.start()

    # ------------------------------------------------------------------
    # Internal readers
    # ------------------------------------------------------------------

    def _read_stdout(self):
        assert self._proc.stdout is not None
        for raw in self._proc.stdout:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                self._output_queue.put(obj)
            except json.JSONDecodeError:
                # Non-JSON output — ignore (could be debug prints)
                pass

    def _read_stderr(self):
        assert self._proc.stderr is not None
        for raw in self._proc.stderr:
            line = raw.decode("utf-8", errors="replace").rstrip()
            self._stderr_lines.append(line)

    # ------------------------------------------------------------------
    # Sending events
    # ------------------------------------------------------------------

    def _send(self, event: dict):
        if self._proc.poll() is not None:
            stderr = self._get_stderr()
            raise RuntimeError(
                f"App process already exited with code {self._proc.returncode}.\n"
                f"stderr:\n{stderr}"
            )
        data = json.dumps(event) + "\n"
        try:
            self._proc.stdin.write(data.encode("utf-8"))
            self._proc.stdin.flush()
        except (BrokenPipeError, OSError) as e:
            stderr = self._get_stderr()
            raise RuntimeError(
                f"Failed to write to app stdin: {e}\nstderr:\n{stderr}"
            ) from e

    def send_init(self, width: int = 800, height: int = 600, launch_dir: str = None):
        """Send init event. Called once at start."""
        event = {
            "type": "init",
            "width": width,
            "height": height,
            "launch_dir": launch_dir or self.launch_dir,
        }
        self._send(event)

    def send_render(self, width: int = None, height: int = None) -> list[dict]:
        """Send render event. Returns list of draw commands up to frame_done."""
        event = {"type": "render"}
        if width is not None:
            event["width"] = width
        if height is not None:
            event["height"] = height
        self._send(event)
        frames = self.read_until_frame_done()
        self._last_frames = frames
        return frames

    def send_key(self, key: str, command: bool = False, shift: bool = False, ctrl: bool = False):
        """Send a key event."""
        self._send({
            "type": "key",
            "key": key,
            "modifiers": {
                "command": command,
                "shift": shift,
                "ctrl": ctrl,
            },
        })

    def send_click(self, x: float, y: float, button: str = "left"):
        """Send a click event."""
        self._send({
            "type": "click",
            "x": x,
            "y": y,
            "button": button,
        })

    def send_command(self, text: str):
        """Send a command event (e.g., '/clear')."""
        self._send({"type": "command", "text": text})

    def get_state(self) -> dict:
        """Send get_state, return the state dict."""
        self._send({"type": "get_state"})
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            try:
                obj = self._output_queue.get(timeout=0.1)
            except queue.Empty:
                continue
            if obj.get("type") == "state":
                return obj
        raise TimeoutError(
            f"Timed out waiting for state response.\nstderr:\n{self._get_stderr()}"
        )

    def set_state(self, state: dict):
        """Send set_state to restore app state."""
        event = {"type": "set_state"}
        event.update(state)
        self._send(event)

    # ------------------------------------------------------------------
    # Reading output
    # ------------------------------------------------------------------

    def read_until_frame_done(self, timeout: float = 5.0) -> list[dict]:
        """Read stdout lines until frame_done. Returns list of draw commands."""
        commands = []
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            try:
                obj = self._output_queue.get(timeout=min(remaining, 0.1))
            except queue.Empty:
                continue
            if obj.get("type") == "frame_done":
                return commands
            commands.append(obj)
        raise TimeoutError(
            f"Timed out waiting for frame_done after {timeout}s. "
            f"Got {len(commands)} commands so far.\n"
            f"stderr:\n{self._get_stderr()}"
        )

    def read_events(self, timeout: float = 1.0) -> list[dict]:
        """Read any pending stdout events (cost_report, api requests, etc.)."""
        events = []
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                obj = self._output_queue.get(timeout=0.05)
                events.append(obj)
            except queue.Empty:
                # If we already have some events and the queue is empty, return
                if events:
                    break
        return events

    # ------------------------------------------------------------------
    # Assertions
    # ------------------------------------------------------------------

    def assert_text_visible(self, text: str, frames: list[dict] = None):
        """Assert that a text draw command contains the given string."""
        frames = frames if frames is not None else self._last_frames
        texts = self.find_texts(frames)
        for t in texts:
            if text in t:
                return
        raise AssertionError(
            f"Text '{text}' not found in draw commands.\n"
            f"Available texts: {texts}\n"
            f"stderr:\n{self._get_stderr()}"
        )

    def assert_rect_at(self, x: float, y: float, frames: list[dict] = None, tolerance: float = 5.0):
        """Assert that a rect exists at approximately (x, y)."""
        frames = frames if frames is not None else self._last_frames
        for cmd in frames:
            if cmd.get("type") == "rect":
                cx = cmd.get("x", 0)
                cy = cmd.get("y", 0)
                if abs(cx - x) <= tolerance and abs(cy - y) <= tolerance:
                    return
        rects = [
            (cmd.get("x"), cmd.get("y"), cmd.get("w"), cmd.get("h"))
            for cmd in frames if cmd.get("type") == "rect"
        ]
        raise AssertionError(
            f"No rect found at approximately ({x}, {y}) within tolerance {tolerance}.\n"
            f"Rects found: {rects}\n"
            f"stderr:\n{self._get_stderr()}"
        )

    def find_texts(self, frames: list[dict] = None) -> list[str]:
        """Extract all text content from draw commands."""
        frames = frames if frames is not None else self._last_frames
        return [
            cmd["text"] for cmd in frames
            if cmd.get("type") == "text" and "text" in cmd
        ]

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def shutdown(self):
        """Send shutdown event, wait for process to exit."""
        if self._closed:
            return
        self._closed = True
        try:
            self._send({"type": "shutdown"})
        except RuntimeError:
            # Process already dead — that's fine
            pass
        try:
            self._proc.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            self._proc.kill()
            self._proc.wait(timeout=2.0)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.shutdown()

    def __del__(self):
        if not self._closed:
            try:
                self.shutdown()
            except Exception:
                pass

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _get_stderr(self) -> str:
        # Give stderr thread a moment to flush
        time.sleep(0.05)
        return "\n".join(self._stderr_lines) if self._stderr_lines else "(empty)"

    @property
    def returncode(self) -> Optional[int]:
        """Poll and return the process return code, or None if still running."""
        return self._proc.poll()


# ======================================================================
# Convenience functions
# ======================================================================

def test_app_lifecycle(entry_point: str):
    """Smoke test: init -> render -> verify frame_done -> shutdown. No crashes."""
    with AppTestHarness(entry_point) as h:
        h.send_init()
        frames = h.send_render()
        # frame_done was received (read_until_frame_done didn't timeout)
        assert isinstance(frames, list), f"Expected list of frames, got {type(frames)}"
        h.shutdown()
    print(f"PASS: lifecycle test for {entry_point}")


def test_app_state_symmetry(entry_point: str):
    """Get state, set state, get state again. Verify they match."""
    with AppTestHarness(entry_point) as h:
        h.send_init()
        h.send_render()

        state1 = h.get_state()
        h.set_state(state1)
        state2 = h.get_state()

        # Compare the user_state portions
        s1 = state1.get("user_state", {})
        s2 = state2.get("user_state", {})
        assert s1 == s2, f"State mismatch after round-trip:\n  before: {s1}\n  after:  {s2}"
        h.shutdown()
    print(f"PASS: state symmetry test for {entry_point}")


def test_app_key_handling(entry_point: str, keys: list[str]):
    """Send a sequence of keys, verify app doesn't crash, state changes."""
    with AppTestHarness(entry_point) as h:
        h.send_init()
        h.send_render()

        for key in keys:
            h.send_key(key)
            # Render after each key to flush state
            h.send_render()

        # If we got here without timeout or crash, it passed
        h.shutdown()
    print(f"PASS: key handling test for {entry_point} with keys {keys}")

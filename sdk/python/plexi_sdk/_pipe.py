from __future__ import annotations

import socket
import struct
import threading
from typing import TYPE_CHECKING, Any

from ._emitter import _emit

if TYPE_CHECKING:
    from ._app import App


# ── Pipe ──────────────────────────────────────────────────────────────────────

class Pipe:
    """Handle for a typed pipe. For binary mode, call connect() after PipeOpened."""

    def __init__(self, pipe_id: str, mode: str, direction: str, app: "App"):
        self.pipe_id = pipe_id
        self.mode = mode
        self.direction = direction
        self._app = app
        self._sock: "socket.socket | None" = None
        self._connected = threading.Event()

    def _on_opened(self, socket_path: str) -> None:
        """Called by App when PipeOpened arrives for this pipe_id."""
        if self.mode == "binary":
            try:
                sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                sock.connect(socket_path)
                self._sock = sock
                self._connected.set()
            except OSError as e:
                self._app.emit.error(f"pipe_open failed pipe_id={self.pipe_id}: {e}")

    def connect(self, timeout: float = 5.0) -> bool:
        """Wait for the pipe to be connected. Returns True on success."""
        return self._connected.wait(timeout=timeout)

    def read_frame(self) -> "bytes | None":
        """Read one length-prefixed frame. Blocks. Returns None on EOF/error/EOS."""
        if not self._sock:
            return None
        try:
            header = self._recv_exact(4)
            if header is None:
                return None
            length = struct.unpack(">I", header)[0]
            if length == 0:
                return None  # EOS sentinel from host
            return self._recv_exact(length)
        except OSError as e:
            self._app.emit.error(f"pipe read_frame error pipe_id={self.pipe_id}: {e}")
            return None

    def write_frame(self, data: bytes) -> None:
        """Write one length-prefixed frame."""
        if not self._sock:
            return
        try:
            header = struct.pack(">I", len(data))
            self._sock.sendall(header + data)
        except OSError as e:
            self._app.emit.error(f"pipe write_frame error pipe_id={self.pipe_id}: {e}")

    def send(self, payload: Any) -> None:
        """JSON-mode pipe send."""
        _emit({"type": "pipe_send", "pipe_id": self.pipe_id, "payload": payload})

    def _recv_exact(self, n: int) -> "bytes | None":
        buf = b""
        while len(buf) < n:
            chunk = self._sock.recv(n - len(buf))  # type: ignore[union-attr]
            if not chunk:
                return None
            buf += chunk
        return buf

    def close(self) -> None:
        if self._sock:
            try:
                # shutdown unblocks any thread blocked in recv() before close().
                self._sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

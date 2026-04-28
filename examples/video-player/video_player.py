#!/usr/bin/env python3
"""Video Player - POC for #345 substrate + #346 AVFoundation backing.

Exercises the host's video subsystem. Two modes:
  - Mock: launch the host with `PLEXI_VIDEO=mock://gradient` and press `o`.
    Source defaults to "mock://gradient", the procedural-gradient decoder
    pumps frames at 30 fps.
  - Real H.264: set `PLEXI_VIDEO_PATH=/abs/path/to/clip.mp4` on the app
    (NOT the host - this app reads it at startup). Press `f` to use the
    file path as the source instead of mock://gradient. Requires #346
    AVFoundation backing on the host.

Substrate flow:
  - emit.open_video(source, pipe_id) -> VideoHandle (handle_id, w/h/fps,
    duration_ms, attached Pipe).
  - A reader thread pulls RGBA8 frames from the Pipe and increments a
    frame counter. The SDK has no rgba-blit primitive yet, so we surface
    "frames decoded" + "last frame size" instead of painting frames.
  - emit.set_video_state(handle_id, "play"|"pause"|"seek", position_ms=...)
    drives playback.
  - emit.close_video(handle_id) tears the decoder down.

Keys:
  o - Open the current source (mock://gradient by default)
  f - Toggle source between mock://gradient and PLEXI_VIDEO_PATH (if set)
  p - Pause
  r - Resume play
  [ - Seek -5s
  ] - Seek +5s
  x - Close the open video
  c - Clear log

Manifest declares `video.playback`. Without it, open_video() raises
CapabilityDeniedError immediately and the app surfaces it in the log.
"""
from __future__ import annotations

import os
import shlex
import sys
import threading
import time

from plexi_sdk import App, RenderContext, VideoHandle, CapabilityDeniedError
from plexi_sdk.ui import (
    AppBar,
    Card,
    Column,
    Footer,
    KeyRow,
    Label,
    ScrollLog,
    Section,
    Spacer,
)


MOCK_SOURCE = "mock://gradient"
# Source resolution priority (most specific wins):
#   1. argv[1]                 — file browser passes the path (#79).
#   2. PLEXI_VIDEO_PATH env    — pre-#79 launch path (kept for the mock harness).
#   3. mock://gradient         — fallback for the substrate POC (#345).
ARGV_SOURCE = sys.argv[1] if len(sys.argv) >= 2 and sys.argv[1] else ""
FILE_SOURCE = ARGV_SOURCE or os.environ.get("PLEXI_VIDEO_PATH", "")
PIPE_ID = "video-stream"
SEEK_STEP_MS = 5_000
MAX_LOG_LINES = 200


class VideoPlayerApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._handle: VideoHandle | None = None
        self._frames_seen: int = 0
        self._last_frame_bytes: int = 0
        self._position_ms: int = 0
        self._state: str = "(closed)"
        # If the file browser launched us with argv[1], default to that
        # source so the user doesn't have to press `f` first. Otherwise
        # the substrate-POC default (mock://gradient) holds.
        self._source: str = FILE_SOURCE if ARGV_SOURCE else MOCK_SOURCE
        self._log_lines: list[str] = []
        self._log_lock = threading.Lock()
        self._reader_stop = threading.Event()
        self._reader_thread: threading.Thread | None = None
        # GUI↔Terminal media bridge (#79): every transport action also
        # emits the equivalent ffplay/ffmpeg invocation through a linked
        # terminal so the user can copy any GUI action as a CLI command.
        self._terminal_pane_id: int = 0
        ctx.status_summary("Video Player - press o to open, f to toggle file/mock, p/r pause/resume, [/] seek")
        self.emit.info("Video Player started")
        if ARGV_SOURCE:
            self.emit.info(f"argv path: {ARGV_SOURCE} (file browser handoff)")
        elif FILE_SOURCE:
            self.emit.info(f"PLEXI_VIDEO_PATH={FILE_SOURCE} - press f to switch to it")
        else:
            self.emit.info("PLEXI_VIDEO_PATH not set - mock://gradient only. Set it to drive AVFoundation.")
        try:
            self._terminal_pane_id = self.emit.request_linked_terminal(
                cwd=None,
                label="video bridge",
            )
            self.emit.info(f"linked terminal pane #{self._terminal_pane_id}")
        except CapabilityDeniedError as e:
            self.emit.warn(f"terminal.bindings denied — CLI bridge disabled: {e}")

    # -- Open / close ----------------------------------------------------------

    # -- CLI bridge (#79) ------------------------------------------------

    def _emit_cli(self, command: str, *, echo: bool = False) -> None:
        """Echo the equivalent CLI through the linked terminal. We use
        ``echo=False`` for the per-action mirror so it doesn't actually
        run alongside the in-app decode — the in-app player IS doing the
        playback. The user sees the command in the terminal as a copy-
        ready primitive. No-op when no terminal is linked."""
        if self._terminal_pane_id == 0:
            return
        # echo=False is observational only — the host always writes
        # command+\n to the PTY (PTY-level echo is shell-controlled);
        # we send a comment line so the user sees the equivalent without
        # actually running ffplay alongside the in-app decode.
        line = command if echo else f"# {command}"
        self.emit.run_in_linked_terminal(self._terminal_pane_id, line, echo=False)

    def _quoted_source(self) -> str:
        # mock:// URLs are fine to pass verbatim to ffplay — it'll
        # error, but the bridge contract is "the equivalent command",
        # not "a command that will succeed".
        return shlex.quote(self._source)

    def _open(self) -> None:
        if self._handle is not None:
            self._append_log("already open - close (x) first")
            return
        self._append_log(f"opening {self._source} ...")
        self._emit_cli(f"ffplay -autoexit {self._quoted_source()}")

        def runner() -> None:
            try:
                handle = self.emit.open_video(source=self._source, pipe_id=PIPE_ID)
                self._handle = handle
                self._state = "play"
                self._position_ms = 0
                self._frames_seen = 0
                self._last_frame_bytes = 0
                self._append_log(
                    f"opened: handle_id={handle.handle_id} "
                    f"{handle.width}x{handle.height} @ {handle.fps:.1f} fps "
                    f"duration_ms={handle.duration_ms}"
                )
                self._spawn_reader(handle)
            except CapabilityDeniedError as e:
                self._append_log(f"capability denied: {e}")
            except RuntimeError as e:
                self._append_log(f"open_video failed: {e}")
            finally:
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    def _spawn_reader(self, handle: VideoHandle) -> None:
        self._reader_stop.clear()

        def reader() -> None:
            if not handle.pipe.connect(timeout=5.0):
                self._append_log("pipe.connect() timed out")
                return
            while not self._reader_stop.is_set():
                frame = handle.pipe.read_frame()
                if frame is None:
                    # EOF - host closed the pipe.
                    break
                self._frames_seen += 1
                self._last_frame_bytes = len(frame)
                # Estimate the playback position from the frame counter so
                # the seekbar in the UI moves even though the mock has no
                # PTS. fps is float; clamp to >0 to avoid div-by-zero.
                if self._state == "play" and handle.fps > 0:
                    self._position_ms = int(
                        (self._frames_seen / handle.fps) * 1000
                    )
                # Repaint at most ~10 fps so the UI doesn't churn.
                if self._frames_seen % max(1, int(handle.fps / 10)) == 0:
                    self.emit.schedule_render(after_ms=20)

        self._reader_thread = threading.Thread(target=reader, daemon=True)
        self._reader_thread.start()
        self.emit.schedule_render(after_ms=20)

    def _close(self) -> None:
        if self._handle is None:
            self._append_log("nothing to close")
            return
        h = self._handle
        self._reader_stop.set()
        self.emit.close_video(h.handle_id)
        self._append_log(f"closed handle_id={h.handle_id}")
        self._emit_cli("ffplay teardown")
        self._handle = None
        self._state = "(closed)"
        self.emit.schedule_render(after_ms=20)

    # -- Playback control ------------------------------------------------------

    def _set_state(self, state: str, position_ms: int = 0) -> None:
        if self._handle is None:
            self._append_log(f"no open video - press o first")
            return
        self.emit.set_video_state(self._handle.handle_id, state, position_ms=position_ms)
        if state == "seek":
            self._state = "play"
            self._position_ms = position_ms
            # Reset the counter so playback-position estimation stays sane
            # after a seek.
            self._frames_seen = int((position_ms / 1000.0) * self._handle.fps)
            self._append_log(f"seek -> {position_ms} ms")
            seconds = position_ms / 1000.0
            self._emit_cli(
                f"ffplay -autoexit -ss {seconds:.3f} {self._quoted_source()}"
            )
        else:
            self._state = state
            self._append_log(f"state -> {state}")
            if state == "pause":
                self._emit_cli("ffplay: press SPACE to toggle pause")
            elif state == "play":
                self._emit_cli(f"ffplay -autoexit {self._quoted_source()}")
        self.emit.schedule_render(after_ms=20)

    # -- Logging ---------------------------------------------------------------

    def _append_log(self, line: str) -> None:
        ts = time.strftime("%H:%M:%S")
        with self._log_lock:
            self._log_lines.append(f"{ts} {line}")
            if len(self._log_lines) > MAX_LOG_LINES:
                del self._log_lines[: len(self._log_lines) - MAX_LOG_LINES]

    # -- Lifecycle -------------------------------------------------------------

    def on_shutdown(self) -> None:
        self._reader_stop.set()
        if self._handle is not None:
            self.emit.close_video(self._handle.handle_id)

    # -- Input -----------------------------------------------------------------

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "o":
            self._open()
        elif k == "f":
            if self._handle is not None:
                self._append_log("close (x) before switching source")
            elif not FILE_SOURCE:
                self._append_log("PLEXI_VIDEO_PATH is not set - relaunch with it pointing to an MP4")
            else:
                self._source = FILE_SOURCE if self._source == MOCK_SOURCE else MOCK_SOURCE
                self._append_log(f"source -> {self._source}")
                self.emit.schedule_render(after_ms=20)
        elif k == "x":
            self._close()
        elif k == "p":
            self._set_state("pause")
        elif k == "r":
            self._set_state("play")
        elif key == "[" or k == "[":
            target = max(0, self._position_ms - SEEK_STEP_MS)
            self._set_state("seek", position_ms=target)
        elif key == "]" or k == "]":
            duration = self._handle.duration_ms if self._handle else 0
            target = min(duration, self._position_ms + SEEK_STEP_MS)
            self._set_state("seek", position_ms=target)
        elif k == "c":
            with self._log_lock:
                self._log_lines.clear()
            self.emit.schedule_render(after_ms=20)

    # -- Render ----------------------------------------------------------------

    def _status_card(self) -> Card:
        rows: list = [Section("Status")]
        if self._handle is None:
            rows.append(Label(f"(closed - press o to open: {self._source})"))
            if FILE_SOURCE and self._source == MOCK_SOURCE:
                rows.append(Label(f"  press f to switch to {FILE_SOURCE}"))
        else:
            h = self._handle
            rows.append(Label(f"  handle_id   {h.handle_id}"))
            rows.append(Label(f"  source      {self._source}"))
            rows.append(Label(f"  size        {h.width} x {h.height}"))
            rows.append(Label(f"  fps         {h.fps:.1f}"))
            rows.append(Label(f"  duration    {h.duration_ms} ms"))
            rows.append(Label(f"  state       {self._state}"))
            rows.append(Label(f"  position    {self._position_ms} / {h.duration_ms} ms"))
            rows.append(Label(f"  frames      {self._frames_seen}"))
            rows.append(Label(f"  last frame  {self._last_frame_bytes} bytes RGBA8"))
        return Card(rows)

    def on_render(self, ctx: RenderContext) -> None:
        with self._log_lock:
            log_snapshot = list(self._log_lines)

        children: list = [
            AppBar(title="Video Player"),
            Label("v3.4 video subsystem (#345 substrate + #346 AVFoundation)"),
            Label("(SDK has no RGBA blit yet - showing frame counter instead.)"),
            self._status_card(),
            Section("Keys"),
            Card([
                KeyRow("o", f"Open {self._source}"),
                KeyRow("f", "Toggle source (mock <-> PLEXI_VIDEO_PATH)"),
                KeyRow("x", "Close current video"),
                KeyRow("p", "Pause"),
                KeyRow("r", "Resume play"),
                KeyRow("[", f"Seek -{SEEK_STEP_MS} ms"),
                KeyRow("]", f"Seek +{SEEK_STEP_MS} ms"),
                KeyRow("c", "Clear log"),
            ]),
            Section(f"Log ({len(log_snapshot)})"),
            Spacer(grow=True),
            ScrollLog(lines=log_snapshot, empty_text="(no events - press o to open)"),
            Footer("Mock: launch host with PLEXI_VIDEO=mock://gradient. AVFoundation: set PLEXI_VIDEO_PATH on the app."),
        ]
        ctx.render(Column(children))


VideoPlayerApp().run()

#!/usr/bin/env python3
"""Video Player - POC for #345.

Exercises the host's video substrate via the procedural mock decoder.
The production AvfVideoDecoder still returns NotImplemented (#346 will
swap in real AVFoundation backing); to drive this app, launch the host
with `PLEXI_VIDEO=mock://gradient` so the factory returns the mock.

Substrate flow:
  - emit.open_video(source, pipe_id) -> VideoHandle (handle_id, w/h/fps,
    duration_ms, attached Pipe).
  - A reader thread pulls RGBA8 frames from the Pipe and increments a
    frame counter. The SDK has no rgba-blit primitive yet, so we surface
    "frames decoded" + "last frame size" instead of painting the gradient.
  - emit.set_video_state(handle_id, "play"|"pause"|"seek", position_ms=...)
    drives playback. The mock pauses / seeks / resumes inside its worker
    thread; the frame counter visibly stalls / resumes.
  - emit.close_video(handle_id) tears the decoder down.

Keys:
  o - Open mock://gradient (requires PLEXI_VIDEO=mock://gradient on host)
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


SOURCE = "mock://gradient"
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
        self._log_lines: list[str] = []
        self._log_lock = threading.Lock()
        self._reader_stop = threading.Event()
        self._reader_thread: threading.Thread | None = None
        ctx.status_summary("Video Player - press o to open, p/r to pause/resume, [/] to seek")
        self.emit.info("Video Player started - launch host with PLEXI_VIDEO=mock://gradient")

    # -- Open / close ----------------------------------------------------------

    def _open(self) -> None:
        if self._handle is not None:
            self._append_log("already open - close (x) first")
            return
        self._append_log(f"opening {SOURCE} ...")

        def runner() -> None:
            try:
                handle = self.emit.open_video(source=SOURCE, pipe_id=PIPE_ID)
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
        else:
            self._state = state
            self._append_log(f"state -> {state}")
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
            rows.append(Label("(closed - press o to open the mock decoder)"))
        else:
            h = self._handle
            rows.append(Label(f"  handle_id   {h.handle_id}"))
            rows.append(Label(f"  source      {SOURCE}"))
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
            Label("v3.4 video substrate (#345) - mock decoder demo"),
            Label("(SDK has no RGBA blit yet - showing frame counter instead.)"),
            self._status_card(),
            Section("Keys"),
            Card([
                KeyRow("o", "Open mock://gradient"),
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
            Footer("Launch host with PLEXI_VIDEO=mock://gradient to drive the mock decoder."),
        ]
        ctx.render(Column(children))


VideoPlayerApp().run()

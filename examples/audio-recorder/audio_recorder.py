#!/usr/bin/env python3
"""Audio Recorder — audio.record + binary pipe proof for PGAP v3.

R to start recording, S to stop and save as recording.wav in workspace_root.
AudioMeter draw command binds to the capture pipe for level display.
Works under PLEXI_AUDIO=mock://fixtures/in.wav,/tmp/out.wav.
"""
from __future__ import annotations

import sys
import os

import pathlib
import struct
import threading
import time
import wave
from plexi_sdk import App, RenderContext, BG, FG, MUTED, ACCENT, SURFACE, RED, GREEN, BODY, CAPTION, HINT

PIPE_ID = "rec"
SAMPLE_RATE = 48000
BUFFER_SIZE = 512


class AudioRecorderApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._state = "IDLE"   # IDLE | RECORDING | SAVED
        self._pipe = None
        self._frames: list[bytes] = []
        self._start_time = 0.0
        self._record_thread: threading.Thread | None = None
        self._stop_flag = threading.Event()

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key == "r" and self._state == "IDLE":
            self._start_recording()
        elif key == "s" and self._state == "RECORDING":
            self._stop_recording()

    def _start_recording(self) -> None:
        self._frames = []
        self._stop_flag.clear()
        self._state = "RECORDING"
        self._start_time = time.time()
        # AudioCapture allocates the binary pipe host-side and emits PipeOpened
        # back to us — don't call pipe_open separately (would collide on pipe_id).
        self._pipe = self.emit.audio_capture(PIPE_ID, sample_rate=SAMPLE_RATE,
                                             buffer_size=BUFFER_SIZE)
        self._record_thread = threading.Thread(
            target=self._capture_loop, daemon=True
        )
        self._record_thread.start()

    def _capture_loop(self) -> None:
        # Wait for pipe to connect (host sends PipeOpened → pipe connects)
        if self._pipe and not self._pipe.connect(timeout=5.0):
            self.emit.error("audio pipe did not open within 5s")
            self._state = "IDLE"
            return
        while not self._stop_flag.is_set():
            if self._pipe:
                frame = self._pipe.read_frame()
                if frame is None:
                    break
                self._frames.append(frame)

    def _stop_recording(self) -> None:
        self._stop_flag.set()
        if self._record_thread:
            self._record_thread.join(timeout=2.0)
        if self._pipe:
            self._pipe.close()
            self._pipe = None
        self._write_wav()
        self._state = "SAVED"
        self.emit.notify(title="Recording saved",
                         body=f"recording.wav ({len(self._frames)} frames)",
                         level="info")

    def _write_wav(self) -> None:
        out_path = pathlib.Path(self.workspace_root) / "recording.wav"
        try:
            with wave.open(str(out_path), "wb") as wf:
                wf.setnchannels(1)
                wf.setsampwidth(2)   # 16-bit
                wf.setframerate(SAMPLE_RATE)
                for raw in self._frames:
                    # Frames arrive as f32 interleaved; convert to i16
                    n_floats = len(raw) // 4
                    floats = struct.unpack(f">{n_floats}f", raw)
                    pcm = struct.pack(
                        f"<{n_floats}h",
                        *[max(-32768, min(32767, int(f * 32767))) for f in floats],
                    )
                    wf.writeframes(pcm)
        except Exception as e:
            self.emit.error(f"wav write failed: {e}")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)
        ctx.rect(0, 0, ctx.w, 44, fill=SURFACE)
        ctx.text(16, 14, "Audio Recorder", size=18.0, color=ACCENT, bold=True)

        # Status
        if self._state == "IDLE":
            status_col = MUTED
            status = "IDLE"
        elif self._state == "RECORDING":
            elapsed = time.time() - self._start_time
            status_col = RED
            status = f"RECORDING  {elapsed:.1f}s"
        else:
            status_col = GREEN
            status = "SAVED  →  recording.wav"

        ctx.text(16, 68, status, size=20.0, color=status_col, bold=True)

        # Audio meter (bound to the capture pipe)
        meter_y = 110.0
        ctx.audio_meter(16, meter_y, ctx.w - 32, 48, PIPE_ID)

        # Help text
        y = ctx.h - 48
        if self._state == "IDLE":
            ctx.text(16, y, "R — start recording", size=HINT, color=MUTED)
        elif self._state == "RECORDING":
            ctx.text(16, y, "S — stop and save", size=HINT, color=MUTED)
        else:
            ctx.text(16, y, "R — record again", size=HINT, color=MUTED)


if __name__ == "__main__":
    AudioRecorderApp().run()

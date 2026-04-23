#!/usr/bin/env python3
"""Audio Recorder — audio.record + binary pipe proof for PGAP v3.

R to start recording, S to stop and save as recording.wav in workspace_root.
AudioMeter draw command binds to the capture pipe for level display.
Works under PLEXI_AUDIO=mock://fixtures/in.wav,/tmp/out.wav.
"""
from __future__ import annotations

import pathlib
import struct
import threading
import time
import wave

from plexi_sdk import App, RenderContext, RED, GREEN, MUTED
from plexi_sdk.ui import (
    Column, Card, Header, Section,
    KeyRow, Label, Spacer, Footer,
)

PIPE_ID = "rec"
SAMPLE_RATE = 48000
BUFFER_SIZE = 512

METER_H = 80.0


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

    # ── Status line ────────────────────────────────────────────────────────
    def _status_text(self) -> str:
        if self._state == "IDLE":
            return "IDLE"
        if self._state == "RECORDING":
            elapsed = time.time() - self._start_time
            return f"RECORDING  {elapsed:.1f}s"
        return "SAVED  →  recording.wav"

    def _status_color(self) -> str:
        if self._state == "IDLE":
            return MUTED
        if self._state == "RECORDING":
            return RED
        return GREEN

    def _footer_text(self) -> str:
        if self._state == "IDLE":
            return "R — start recording"
        if self._state == "RECORDING":
            return "S — stop and save"
        return "R — record again"

    # ── Audio meter — primitive escape hatch ───────────────────────────────
    class _Meter:
        """Custom component: binds the host-side AudioMeter to the pipe."""
        HEIGHT = METER_H

        def measure(self, avail_w: float) -> float:
            return self.HEIGHT

        def is_grow(self) -> bool:
            return False

        def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
            ctx.audio_meter(x, y + (h - 48.0) / 2.0, w, 48.0, PIPE_ID)

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            Header(
                title="Audio Recorder",
                subtitle="Capture mic input to recording.wav",
            ),
            Card([
                KeyRow("r", "Start recording"),
                KeyRow("s", "Stop and save"),
            ]),
            Section("Status"),
            Label(self._status_text(), tone="body",
                  color=self._status_color(), bold=True),
            Section("Level"),
            self._Meter(),
            Spacer(grow=True),
            Footer(self._footer_text()),
        ]))


if __name__ == "__main__":
    AudioRecorderApp().run()

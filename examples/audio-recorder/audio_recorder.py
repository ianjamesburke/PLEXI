#!/usr/bin/env python3
"""Audio Recorder — POC for #341.

Demonstrates emit.list_audio_devices() + audio capture + WAV export.
Keys: left/right to select device, r to record/stop, s to save WAV.
"""
from __future__ import annotations
import array
import struct
import sys
import threading
import wave
import pathlib
from typing import TYPE_CHECKING

from plexi_sdk import App, RenderContext, CapabilityDeniedError

if TYPE_CHECKING:
    from plexi_sdk import AudioDeviceInfo

PIPE_ID = "audio-recorder-mic"
SAMPLE_RATE = 48_000
CHANNELS = 1
BUFFER_SIZE = 512


class AudioRecorderApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._devices: list[AudioDeviceInfo] = []
        self._device_idx: int = 0
        self._capturing: bool = False
        self._pipe = None
        self._reader_thread: threading.Thread | None = None
        self._reader_stop = threading.Event()
        self._frames: list[bytes] = []   # raw f32 LE bytes
        self._frames_lock = threading.Lock()
        self._peak: float = 0.0
        self._status: str = "Loading devices..."
        self.emit.info("audio-recorder: starting")

        try:
            device_list = await self.emit.list_audio_devices()
            self._devices = device_list.inputs
            self._status = f"Ready — {len(self._devices)} input device(s) found"
            self.emit.info(f"audio-recorder: {len(self._devices)} input devices")
        except Exception as e:
            self._status = f"Device list failed: {e}"
            self.emit.error(str(e))

        ctx.status_summary("Audio Recorder")

    def _selected_device(self) -> AudioDeviceInfo | None:
        if not self._devices:
            return None
        return self._devices[self._device_idx % len(self._devices)]

    def _start_capture(self) -> None:
        if self._capturing:
            return
        dev = self._selected_device()
        device_id = dev.id if dev else None
        with self._frames_lock:
            self._frames.clear()
        self._reader_stop.clear()
        try:
            self._pipe = self.emit.audio_capture(
                pipe_id=PIPE_ID,
                sample_rate=SAMPLE_RATE,
                buffer_size=BUFFER_SIZE,
                device_id=device_id,
            )
        except CapabilityDeniedError as e:
            self._status = f"Denied: {e}"
            self.emit.error(str(e))
            return
        self._capturing = True
        self._status = "Recording..."
        self._reader_thread = threading.Thread(target=self._reader, daemon=True)
        self._reader_thread.start()
        self.emit.info(f"audio-recorder: capture started device_id={device_id!r}")

    def _stop_capture(self) -> None:
        if not self._capturing:
            return
        self._reader_stop.set()
        if self._pipe:
            try:
                self._pipe.close()
            except Exception:
                pass
            self._pipe = None
        if self._reader_thread:
            self._reader_thread.join(timeout=2.0)
            self._reader_thread = None
        self._capturing = False
        with self._frames_lock:
            n = len(self._frames)
        self._status = f"Stopped — {n} frame chunks captured. Press s to save."
        self.emit.info(f"audio-recorder: capture stopped, {n} chunks")

    def _reader(self) -> None:
        """Background thread: read f32 PCM frames from the binary pipe."""
        if self._pipe is None:
            return
        if not self._pipe.connect(timeout=5.0):
            self.emit.warn("audio-recorder: pipe connect timed out")
            return
        while not self._reader_stop.is_set():
            frame = self._pipe.read_frame()
            if frame is None:
                break
            count = len(frame) // 4
            if count > 0:
                floats = struct.unpack_from(f"<{count}f", frame)
                self._peak = max(abs(s) for s in floats)
                with self._frames_lock:
                    self._frames.append(frame)
                self.emit.schedule_render(after_ms=50)

    def _save_wav(self) -> None:
        with self._frames_lock:
            all_data = b"".join(self._frames)
        if not all_data:
            self._status = "Nothing to save."
            return
        path = pathlib.Path.home() / "Desktop" / "recording.wav"
        n_floats = len(all_data) // 4
        samples = struct.unpack_from(f"<{n_floats}f", all_data)
        arr = array.array("h", (int(max(-1.0, min(1.0, s)) * 32767) for s in samples))
        if sys.byteorder == "big":
            arr.byteswap()
        pcm_i16 = arr.tobytes()
        with wave.open(str(path), "wb") as wf:
            wf.setnchannels(CHANNELS)
            wf.setsampwidth(2)  # 16-bit
            wf.setframerate(SAMPLE_RATE)
            wf.writeframes(pcm_i16)
        duration = len(samples) / SAMPLE_RATE
        self._status = f"Saved: {path} ({duration:.1f}s)"
        self.emit.info(f"audio-recorder: saved {path} ({duration:.1f}s)")

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key in ("left", "[") and not self._capturing:
            self._device_idx = (self._device_idx - 1) % max(1, len(self._devices))
        elif key in ("right", "]") and not self._capturing:
            self._device_idx = (self._device_idx + 1) % max(1, len(self._devices))
        elif key == "r":
            if self._capturing:
                self._stop_capture()
            else:
                self._start_capture()
        elif key == "s" and not self._capturing:
            self._save_wav()
        self.emit.schedule_render(after_ms=20)

    def on_render(self, ctx: RenderContext) -> None:
        PAD = 20
        y = PAD

        # Title
        ctx.text(PAD, y, "Audio Recorder", size=18.0, color="#cdd6f4", bold=True)
        y += 30

        # Device selector
        dev = self._selected_device()
        n_dev = len(self._devices)
        dev_idx_label = f" ({self._device_idx + 1}/{n_dev})" if n_dev > 1 else ""
        dev_name = (dev.name if dev else "(no devices)") + dev_idx_label
        ctx.text(PAD, y, "Device:", size=13.0, color="#a6adc8")
        ctx.text(PAD + 65, y, dev_name, size=13.0, color="#cdd6f4")
        if not self._capturing:
            ctx.text(PAD, y + 18, "← → or [ ] to change device", size=11.0, color="#585b70")
        y += 45

        # Peak meter
        meter_w = ctx.w - PAD * 2
        meter_h = 18.0
        ctx.rect(PAD, y, meter_w, meter_h, fill="#1e1e2e", radius=3.0)
        peak = self._peak
        fill_w = meter_w * min(peak, 1.0)
        if fill_w > 0:
            color = "#50c878" if peak < 0.6 else "#f9e2af" if peak < 0.85 else "#f38ba8"
            ctx.rect(PAD, y, fill_w, meter_h, fill=color, radius=3.0)
        ctx.text(PAD, y + meter_h + 4, f"Peak: {peak:.3f}", size=11.0, color="#585b70")
        y += meter_h + 22

        # Record button
        rec_color = "#f38ba8" if self._capturing else "#a6e3a1"
        rec_label = "Stop (r)" if self._capturing else "Record (r)"
        ctx.rect(PAD, y, 120, 28, fill=rec_color, radius=5.0)
        ctx.text(PAD + 8, y + 7, rec_label, size=13.0, color="#1e1e2e")
        y += 40

        # Save button (only when not capturing)
        if not self._capturing:
            ctx.rect(PAD, y, 120, 28, fill="#89b4fa", radius=5.0)
            ctx.text(PAD + 8, y + 7, "Save WAV (s)", size=13.0, color="#1e1e2e")
            y += 40

        # Status
        ctx.text(PAD, y, self._status, size=12.0, color="#a6adc8")


if __name__ == "__main__":
    AudioRecorderApp().run()

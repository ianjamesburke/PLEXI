#!/usr/bin/env python3
"""Audio Bridge Test — POC for #79 (GUI↔Terminal media bridge).

End-to-end demo of the v3.4 release-level checklist item:
"Run the GUI↔terminal media bridge demo: terminal plays an audio file,
the canvas pane visualizes the waveform in real time."

Mirror of the goal but driven by mic capture so the demo is self-
contained — no test asset needed:
  - Records: starts mic capture (#277), reads f32 PCM frames from the
    binary pipe, computes peak meters per ~50 ms window, and paints
    them across the pane.
  - Echoes the equivalent ffmpeg command through a linked terminal
    so the user sees how to do the same thing from CLI.

Capabilities: ``audio.record`` (#277) + ``terminal.bindings`` (#78).

Keys:
  r - Record (start capture + echo ffmpeg command)
  s - Stop (end capture + echo "# capture stopped")
  c - Clear waveform
"""

import struct
import threading

from plexi_sdk import App, RenderContext, CapabilityDeniedError


PIPE_ID = "audio-bridge-mic"
SAMPLE_RATE = 48000
BUFFER_SIZE = 512
# Last ~5 s of peak data, one peak per ~50 ms window. 5000 / 50 = 100.
MAX_PEAKS = 100
PEAK_WINDOW_MS = 50

# Equivalent CLI: macOS avfoundation default mic, 5s capture, 48 kHz wav.
FFMPEG_CAPTURE = (
    "ffmpeg -f avfoundation -i :0 -t 5 -ar 48000 capture.wav"
)


class AudioBridgeTestApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._terminal_pane_id: int = 0
        self._capturing: bool = False
        self._reader_thread: "threading.Thread | None" = None
        self._reader_stop = threading.Event()
        self._pipe = None
        # Rolling window of recent peaks. Each value is the peak |sample|
        # in the last PEAK_WINDOW_MS window, clamped to [0, 1].
        self._peaks: list[float] = []
        self._peaks_lock = threading.Lock()
        # Running window state.
        self._window_peak: float = 0.0
        self._window_samples: int = 0
        self._frames_seen: int = 0

        ctx.status_summary("Audio Bridge Test — press r to record")
        self.emit.info("audio-bridge-test starting")
        try:
            self._terminal_pane_id = await self.emit.request_linked_terminal(
                cwd=None,
                label="audio bridge",
            )
            self.emit.info(f"linked terminal pane #{self._terminal_pane_id}")
        except CapabilityDeniedError as e:
            self.emit.error(str(e))

    # -- Bridge ---------------------------------------------------------

    def _emit_cli(self, command: str, *, echo: bool = True) -> None:
        if self._terminal_pane_id == 0:
            return
        self.emit.run_in_linked_terminal(
            self._terminal_pane_id, command, echo=echo,
        )

    # -- Capture --------------------------------------------------------

    def _start(self) -> None:
        if self._capturing:
            return
        self._capturing = True
        self._reader_stop.clear()
        try:
            self._pipe = self.emit.audio_capture(
                pipe_id=PIPE_ID,
                sample_rate=SAMPLE_RATE,
                buffer_size=BUFFER_SIZE,
            )
        except CapabilityDeniedError as e:
            self._capturing = False
            self.emit.error(str(e))
            return
        self._emit_cli(FFMPEG_CAPTURE, echo=True)
        self._reader_thread = threading.Thread(
            target=self._reader, daemon=True
        )
        self._reader_thread.start()
        self.emit.schedule_render(after_ms=20)

    def _stop(self) -> None:
        if not self._capturing:
            return
        self._capturing = False
        self._reader_stop.set()
        # Comment-only — doesn't run anything destructive.
        self._emit_cli("# capture stopped", echo=True)
        self.emit.schedule_render(after_ms=20)

    def _reader(self) -> None:
        """Pull f32 PCM packets from the binary pipe and accumulate the
        rolling peak window. Each packet is a tightly-packed array of
        f32 samples (interleaved if multi-channel — we treat them as a
        flat stream for the waveform, which over-counts per-frame width
        but doesn't matter for a peak meter)."""
        if self._pipe is None:
            return
        if not self._pipe.connect(timeout=5.0):
            self.emit.warn("audio-bridge-test: pipe connect timed out")
            return
        window_samples_target = int(SAMPLE_RATE * PEAK_WINDOW_MS / 1000)
        while not self._reader_stop.is_set():
            frame = self._pipe.read_frame()
            if frame is None:
                break
            self._frames_seen += 1
            # Decode f32 little-endian samples. Slice in 4-byte chunks
            # without an explicit numpy dep so the SDK install stays
            # vanilla.
            count = len(frame) // 4
            if count == 0:
                continue
            samples = struct.unpack_from(f"<{count}f", frame)
            for s in samples:
                a = abs(s)
                if a > self._window_peak:
                    self._window_peak = a
                self._window_samples += 1
                if self._window_samples >= window_samples_target:
                    with self._peaks_lock:
                        self._peaks.append(min(1.0, self._window_peak))
                        if len(self._peaks) > MAX_PEAKS:
                            del self._peaks[: len(self._peaks) - MAX_PEAKS]
                    self._window_peak = 0.0
                    self._window_samples = 0
            # Repaint at most ~20 fps.
            if self._frames_seen % 4 == 0:
                self.emit.schedule_render(after_ms=20)

    # -- Input ----------------------------------------------------------

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "r":
            self._start()
        elif k == "s":
            self._stop()
        elif k == "c":
            with self._peaks_lock:
                self._peaks.clear()
            self.emit.schedule_render(after_ms=16)

    # -- Render ---------------------------------------------------------

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h

        bg = "#0e0e10"
        fg = "#e2e8f0"
        muted = "#64748b"
        accent = "#34d399"
        red = "#ef4444"
        ctx.rect(0, 0, w, h, bg)

        # Header.
        ctx.text(x=24, y=24, text="audio bridge test",
                 size=22.0, color=fg, bold=True)
        if self._terminal_pane_id == 0:
            sub = "no linked terminal — manifest missing terminal.bindings"
            sub_col = red
        else:
            sub = f"linked terminal #{self._terminal_pane_id}  ·  r record  ·  s stop  ·  c clear"
            sub_col = muted
        ctx.text(x=24, y=58, text=sub, size=12.0, color=sub_col)

        # State badge.
        state_text = "RECORDING" if self._capturing else "idle"
        state_col = red if self._capturing else muted
        ctx.text(x=24, y=86, text=state_text, size=14.0, color=state_col,
                 monospace=True, bold=True)

        # Waveform area.
        wf_x = 24
        wf_y = 110
        wf_w = max(w - 48, 200)
        wf_h = max(h - wf_y - 80, 80)
        ctx.rect(wf_x, wf_y, wf_w, wf_h, "#1a1a1f", radius=6.0)
        midline = wf_y + wf_h / 2
        ctx.line(wf_x, midline, wf_x + wf_w, midline, muted, width=1.0)

        with self._peaks_lock:
            peaks = list(self._peaks)
        if peaks:
            bar_w = max(1.0, wf_w / MAX_PEAKS)
            offset = wf_w - len(peaks) * bar_w  # right-align (newest on right)
            for i, p in enumerate(peaks):
                bar_h = p * (wf_h / 2 - 4)
                x = wf_x + offset + i * bar_w
                ctx.rect(x, midline - bar_h, bar_w * 0.85, bar_h * 2,
                         accent)
        else:
            ctx.text(x=wf_x + 12, y=midline, text="(no audio yet — press r)",
                     size=12.0, color=muted, align="left_center")

        # Footer: equivalent CLI.
        ctx.text(x=24, y=h - 48,
                 text="equivalent CLI (echoed in terminal on record):",
                 size=11.0, color=muted)
        ctx.text(x=24, y=h - 26, text=f"$ {FFMPEG_CAPTURE}",
                 size=12.0, color=fg, monospace=True,
                 max_width=w - 48, elide=True)


if __name__ == "__main__":
    AudioBridgeTestApp().run()

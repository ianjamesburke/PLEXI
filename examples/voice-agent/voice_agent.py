#!/usr/bin/env python3
"""Voice Agent — PGAP app (#888).

Mic → VAD → Whisper → ai.query (tool loop) → TTS → AudioPlay.
No host Rust changes required. Dependencies: requests>=2.28.
"""
from __future__ import annotations

import asyncio
import io
import json
import os
import struct
import subprocess
import tempfile
import threading
import urllib.error
import urllib.request
import uuid
import wave
from typing import Optional

from plexi_sdk import App, RenderContext, CapabilityDeniedError

PIPE_ID = "voice-agent-mic"
SAMPLE_RATE = 48_000
BUFFER_SIZE = 512

VAD_THRESHOLD = 0.03       # raised from 0.01 — typical MacBook ambient noise ~0.01-0.02
SPEECH_START_FRAMES = 5   # 100ms at 16kHz/20ms
SPEECH_END_FRAMES = 35    # 700ms at 16kHz/20ms
MIN_TURN_FRAMES = 12      # 250ms at 16kHz/20ms
MAX_TURN_FRAMES = 1500    # 30s hard cap — prevents VAD sticking if silence never arrives

SYSTEM_PROMPT = (
    "You are Plexi's voice agent. The user's speech is transcribed and given to you. "
    "Use the provided tools to control their Plexi workspace. Be extremely concise. "
    "After a tool call, confirm in one short sentence. If the request is ambiguous, "
    "ask one clarifying question rather than guessing."
)


class VoiceAgentApp(App):

    async def on_init(self, ctx: RenderContext) -> None:
        self._status = "Starting..."
        self._history: list[dict] = []
        self._turn_queue: asyncio.Queue[bytes] = asyncio.Queue()
        self._pipe = None
        self._reader_stop = threading.Event()
        self._reader_thread: Optional[threading.Thread] = None
        self._temp_files: list[str] = []
        self._process_task: Optional[asyncio.Task] = None

        self._register_tools()

        ctx.status_summary("Voice Agent")
        self.emit.info("voice-agent: starting")

        try:
            self._pipe = self.emit.audio_capture(
                pipe_id=PIPE_ID,
                sample_rate=SAMPLE_RATE,
                buffer_size=BUFFER_SIZE,
            )
            self._reader_stop.clear()
            # Capture the event loop to avoid Optional access in the thread.
            loop = asyncio.get_running_loop()
            self._reader_thread = threading.Thread(
                target=self._vad_loop, args=(loop,), daemon=True
            )
            self._reader_thread.start()
            self._status = "Listening..."
            self.emit.info(f"voice-agent: audio capture started sample_rate={SAMPLE_RATE}")
        except CapabilityDeniedError as e:
            self._status = f"Mic denied: {e}"
            self.emit.error(f"voice-agent: mic denied: {e}")
            return

        self._process_task = asyncio.create_task(self._process_loop())

    def _register_tools(self) -> None:
        @self.tool(
            "pane_open",
            description="Open a new pane. type_id: 'terminal' or an installed app id.",
            schema={
                "type": "object",
                "properties": {
                    "type_id": {"type": "string", "description": "Pane type to open (e.g. 'terminal')"},
                },
                "required": ["type_id"],
            },
        )
        def handle_pane_open(args: dict) -> dict:
            return self._run_plexi(["open", args["type_id"]])

        @self.tool(
            "pane_list",
            description="List all open panes as JSON. Returns pane IDs and titles needed for pane_close and pane_focus.",
            schema={"type": "object", "properties": {}},
        )
        def handle_pane_list(_args: dict) -> dict:
            return self._run_plexi(["pane", "list"])

        @self.tool(
            "pane_close",
            description="Close a pane by its numeric ID. Call pane_list first to get IDs.",
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {"type": "integer", "description": "Pane ID from pane_list"},
                },
                "required": ["pane_id"],
            },
        )
        def handle_pane_close(args: dict) -> dict:
            return self._run_plexi(["pane", "close", str(args["pane_id"])])

        @self.tool(
            "pane_focus",
            description="Move UI focus to a pane by its numeric ID. Call pane_list first to get IDs.",
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {"type": "integer", "description": "Pane ID from pane_list"},
                },
                "required": ["pane_id"],
            },
        )
        def handle_pane_focus(args: dict) -> dict:
            return self._run_plexi(["pane", "focus", str(args["pane_id"])])

        @self.tool(
            "notify",
            description="Send a desktop notification with a message.",
            schema={
                "type": "object",
                "properties": {
                    "message": {"type": "string", "description": "Notification text"},
                },
                "required": ["message"],
            },
        )
        def handle_notify(args: dict) -> dict:
            return self._run_plexi(["notify", args["message"]])

        @self.tool(
            "run",
            description="Run a named command from .plexi/commands.toml.",
            schema={
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Command name to run"},
                },
                "required": ["name"],
            },
        )
        def handle_run(args: dict) -> dict:
            return self._run_plexi(["run", args["name"]])

    def _run_plexi(self, args: list[str]) -> dict:
        """Run a plexi subcommand via subprocess. PLEXI_SOCKET is inherited."""
        cmd = ["plexi"] + args
        self.emit.info(f"voice-agent: tool {' '.join(args[:2])} args={args[2:]}")
        try:
            result = subprocess.run(
                cmd, capture_output=True, text=True, timeout=10, env=os.environ
            )
            out = (result.stdout + result.stderr).strip()[:4096]
            self.emit.info(f"voice-agent: tool {args[0]} exit={result.returncode} out={len(out)}b")
            return {"output": out or "(ok)", "exit_code": result.returncode}
        except subprocess.TimeoutExpired:
            self.emit.warn(f"voice-agent: tool {args[0]} timed out")
            return {"output": "(timeout)", "exit_code": -1}
        except Exception as e:
            self.emit.error(f"voice-agent: tool {args[0]} error: {e}")
            return {"output": str(e), "exit_code": -1}

    def _vad_loop(self, loop: asyncio.AbstractEventLoop) -> None:
        """Background thread: pipe → VAD → complete turns → queue."""
        if self._pipe is None:
            return
        if not self._pipe.connect(timeout=5.0):
            self.emit.warn("voice-agent: pipe connect timed out")
            self._status = "Mic connect timeout"
            return

        self.emit.info("voice-agent: pipe connected, VAD active")

        # Decimate 48kHz → 16kHz (factor 3). Works for mono; for stereo the
        # interleaved samples produce every-other-channel decimation which is
        # still adequate for speech VAD and Whisper transcription.
        DECIMATE = SAMPLE_RATE // 16_000
        FRAME_LEN = 320  # 20ms at 16kHz

        pcm_buf: list[int] = []     # current turn i16 samples
        frame_buf: list[float] = [] # f32 accumulator at 16kHz

        speaking = False
        silent_frames = 0
        speech_frames = 0

        while not self._reader_stop.is_set():
            frame = self._pipe.read_frame()
            if frame is None:
                break

            count = len(frame) // 4
            if count == 0:
                continue

            floats = struct.unpack_from(f"<{count}f", frame)
            # Decimate to 16kHz without channel mixing to avoid complexity.
            decimated = floats[::DECIMATE]
            frame_buf.extend(decimated)

            while len(frame_buf) >= FRAME_LEN:
                vad_frame = frame_buf[:FRAME_LEN]
                frame_buf = frame_buf[FRAME_LEN:]

                rms = (sum(s * s for s in vad_frame) / FRAME_LEN) ** 0.5

                if rms > VAD_THRESHOLD:
                    silent_frames = 0
                    speech_frames += 1
                    if not speaking and speech_frames >= SPEECH_START_FRAMES:
                        speaking = True
                        self.emit.info(f"voice-agent: speech start detected rms={rms:.4f}")
                        self._status = "Hearing..."
                        self.emit.schedule_render(after_ms=50)
                    if speaking:
                        pcm_buf.extend(
                            int(max(-1.0, min(1.0, s)) * 32767) for s in vad_frame
                        )
                        if len(pcm_buf) >= MAX_TURN_FRAMES * FRAME_LEN:
                            self.emit.info(
                                f"voice-agent: max turn length reached ({MAX_TURN_FRAMES} frames), flushing"
                            )
                            silent_frames = SPEECH_END_FRAMES  # force end-of-turn path below
                else:
                    if speaking:
                        silent_frames += 1
                        pcm_buf.extend(
                            int(max(-1.0, min(1.0, s)) * 32767) for s in vad_frame
                        )
                        if silent_frames >= SPEECH_END_FRAMES:
                            turn_samples = len(pcm_buf)
                            if turn_samples >= MIN_TURN_FRAMES * FRAME_LEN:
                                wav_bytes = _encode_wav(pcm_buf)
                                self.emit.info(
                                    f"voice-agent: turn complete {turn_samples} samples → queue"
                                )
                                loop.call_soon_threadsafe(
                                    self._turn_queue.put_nowait, wav_bytes
                                )
                                self._status = "Processing..."
                                self.emit.schedule_render(after_ms=50)
                            else:
                                self.emit.info("voice-agent: turn too short, discarding")
                            pcm_buf = []
                            speaking = False
                            speech_frames = 0
                            silent_frames = 0
                    else:
                        speech_frames = max(0, speech_frames - 1)

        self.emit.info("voice-agent: VAD loop exited")

    async def _process_loop(self) -> None:
        """Async loop: drain turn queue → transcribe → AI → TTS."""
        while True:
            wav_bytes = await self._turn_queue.get()
            try:
                await self._process_turn(wav_bytes)
            except asyncio.CancelledError:
                raise
            except Exception as e:
                self.emit.error(f"voice-agent: process_turn error: {e}")
                self._status = f"Error: {e}"
                self.emit.schedule_render(after_ms=100)
            finally:
                self._status = "Listening..."
                self.emit.schedule_render(after_ms=100)

    async def _process_turn(self, wav_bytes: bytes) -> None:
        api_key = os.environ.get("OPENAI_API_KEY", "")
        if not api_key:
            self.emit.error("voice-agent: OPENAI_API_KEY not set — inject via plexi secret set")
            self._status = "Error: OPENAI_API_KEY not set"
            return

        # 1. Transcribe
        self._status = "Transcribing..."
        self.emit.schedule_render(after_ms=50)
        loop = asyncio.get_event_loop()
        try:
            text = await loop.run_in_executor(None, _transcribe, wav_bytes, api_key)
        except Exception as e:
            self.emit.error(f"voice-agent: transcription failed: {e}")
            return
        if not text:
            self.emit.info("voice-agent: empty transcription, skipping")
            return

        self.emit.info(f"voice-agent: heard: {text}")
        self._status = f"Heard: {text[:50]}"
        self.emit.schedule_render(after_ms=50)

        # 2. AI query (broker handles full tool-call loop)
        self._history.append({"role": "user", "content": text})
        self._status = "Thinking..."
        self.emit.schedule_render(after_ms=50)

        try:
            response = await self.emit.ai_query(
                model_tier="medium",
                system=SYSTEM_PROMPT,
                messages=self._history,
            )
        except Exception as e:
            self.emit.error(f"voice-agent: ai_query failed: {e}")
            self._history.pop()  # remove the user turn that failed
            return

        reply = (response.content or "").strip()
        if reply:
            self._history.append({"role": "assistant", "content": reply})
        self.emit.info(f"voice-agent: AI replied: {reply[:120]}")

        # 3. TTS + play
        if reply:
            self._status = "Speaking..."
            self.emit.schedule_render(after_ms=50)
            try:
                tmp_path = await loop.run_in_executor(None, _tts, reply, api_key)
                self._temp_files.append(tmp_path)
                self.emit.info(f"voice-agent: playing TTS {tmp_path}")
                self.emit.audio_play(tmp_path)  # type: ignore[attr-defined]
            except Exception as e:
                self.emit.error(f"voice-agent: TTS failed: {e}")

    def on_shutdown(self) -> None:
        self.emit.info("voice-agent: shutting down")
        if self._process_task:
            self._process_task.cancel()
        self._reader_stop.set()
        if self._pipe:
            try:
                self._pipe.close()
            except Exception:
                pass
        if self._reader_thread:
            self._reader_thread.join(timeout=2.0)
        for p in self._temp_files:
            try:
                os.unlink(p)
            except Exception:
                pass
        self.emit.info("voice-agent: shutdown complete")

    def on_render(self, ctx: RenderContext) -> None:
        PAD = 20
        y = PAD

        ctx.text(PAD, y, "Voice Agent", size=18.0, color="#cdd6f4", bold=True)
        y += 32

        # Status line with colour coding
        if "Listening" in self._status:
            status_color = "#a6e3a1"
        elif "Hearing" in self._status or "Transcrib" in self._status:
            status_color = "#f9e2af"
        elif "Error" in self._status:
            status_color = "#f38ba8"
        else:
            status_color = "#89b4fa"

        ctx.text(PAD, y, self._status, size=13.0, color=status_color)
        y += 28

        # Conversation preview (last 6 messages)
        dim = "#585b70"
        for msg in self._history[-6:]:
            role_label = "You:" if msg["role"] == "user" else "Plexi:"
            role_color = "#a6adc8" if msg["role"] == "user" else "#89dceb"
            snippet = msg["content"][:55] + ("…" if len(msg["content"]) > 55 else "")
            ctx.text(PAD, y, role_label, size=11.0, color=role_color, bold=True)
            ctx.text(PAD + 48, y, snippet, size=11.0, color=dim)
            y += 17

        if not self._history:
            ctx.text(PAD, y, "Speak to control Plexi.", size=12.0, color=dim)


# ── Module-level helpers (no self) ─────────────────────────────────────────

def _encode_wav(pcm_i16: list[int]) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16_000)
        w.writeframes(struct.pack(f"<{len(pcm_i16)}h", *pcm_i16))
    return buf.getvalue()


def _multipart(fields: dict, files: dict) -> tuple[str, bytes]:
    """Build a multipart/form-data body without external dependencies."""
    boundary = uuid.uuid4().hex
    body = io.BytesIO()
    for name, value in fields.items():
        body.write(f"--{boundary}\r\n".encode())
        body.write(f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode())
        body.write(f"{value}\r\n".encode())
    for name, (filename, data, mimetype) in files.items():
        body.write(f"--{boundary}\r\n".encode())
        body.write(
            f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\n'
            f"Content-Type: {mimetype}\r\n\r\n".encode()
        )
        body.write(data)
        body.write(b"\r\n")
    body.write(f"--{boundary}--\r\n".encode())
    return f"multipart/form-data; boundary={boundary}", body.getvalue()


def _transcribe(wav_bytes: bytes, api_key: str) -> str:
    content_type, body = _multipart(
        {"model": "whisper-1", "response_format": "json"},
        {"file": ("turn.wav", wav_bytes, "audio/wav")},
    )
    req = urllib.request.Request(
        "https://api.openai.com/v1/audio/transcriptions",
        data=body,
        headers={"Authorization": f"Bearer {api_key}", "Content-Type": content_type},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.loads(resp.read()).get("text", "").strip()


def _tts(text: str, api_key: str) -> str:
    """Synthesise text to speech. Returns path to temp mp3 file."""
    body = json.dumps(
        {"model": "tts-1", "voice": "alloy", "input": text[:4096]}
    ).encode()
    req = urllib.request.Request(
        "https://api.openai.com/v1/audio/speech",
        data=body,
        headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        audio_data = resp.read()
    with tempfile.NamedTemporaryFile(suffix=".mp3", delete=False) as f:
        f.write(audio_data)
        return f.name


if __name__ == "__main__":
    VoiceAgentApp().run()

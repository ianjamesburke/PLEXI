#!/usr/bin/env python3
"""MIDI Test — POC for #320.

Exercises the host's CoreMIDI broker:
  - Enumerates input + output ports via emit.list_midi_devices()
  - Click an input port to open a stream; incoming MIDI messages render in the
    log column with hex bytes + decoded form (NoteOn ch=N note=M vel=V).
  - "Send NoteOn" / "Send NoteOff" buttons on each output dispatch middle C
    (note 60) at velocity 100 — useful for verifying an external synth.
  - "Send Clock pulse" sends a single MIDI clock byte (0xF8) — useful for
    verifying that an external sequencer set to external clock receives the
    pulse.

Keys:
  r — Refresh the device list
  c — Clear the message log
  o — Open the first available input
  x — Close the currently-open input
  n — Send NoteOn (middle C, vel 100) to the first output
  m — Send NoteOff (middle C) to the first output
  k — Send a single clock pulse (0xF8) to the first output

The host requires `midi.in` and `midi.out` declared in manifest.toml.
"""
from __future__ import annotations

import threading
import time

from plexi_sdk import App, RenderContext, MidiDeviceList, MidiPortInfo
from plexi_sdk import midi as midi_lib
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


MAX_LOG_LINES = 200
INPUT_PIPE_ID = "midi-test-input"


class MidiTestApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        # Device list cache populated via emit.list_midi_devices(). None until
        # the first refresh completes; empty list after a refresh that returns
        # zero devices.
        self._devices: MidiDeviceList | None = None
        self._refresh_in_flight = False
        self._open_port_id: str | None = None
        self._open_port_name: str | None = None
        self._log_lines: list[str] = []
        self._log_lock = threading.Lock()
        self._reader_thread: threading.Thread | None = None
        self._reader_stop = threading.Event()
        ctx.status_summary("MIDI Test — press r to refresh, o to open first input")
        self.emit.info("MIDI Test started")
        self._refresh_devices()

    # ── Device enumeration ─────────────────────────────────────────────────

    def _refresh_devices(self) -> None:
        if self._refresh_in_flight:
            return
        self._refresh_in_flight = True
        self.emit.schedule_render(after_ms=20)

        def runner() -> None:
            try:
                devs = self.emit.run_sync(self.emit.list_midi_devices())
                self._devices = devs
                self._append_log(
                    f"refreshed: {len(devs.inputs)} input(s), {len(devs.outputs)} output(s)"
                )
            except Exception as e:
                self._append_log(f"list_midi_devices failed: {e}")
                self.emit.warn(f"list_midi_devices failed: {e}")
            finally:
                self._refresh_in_flight = False
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    # ── Input streaming ────────────────────────────────────────────────────

    def _open_first_input(self) -> None:
        if self._open_port_id is not None:
            self._append_log(
                f"already open on '{self._open_port_name}' — close (x) first"
            )
            return
        if self._devices is None or not self._devices.inputs:
            self._append_log("no inputs to open — refresh (r) first")
            return
        self._open_input(self._devices.inputs[0])

    def _open_input(self, port: MidiPortInfo) -> None:
        if self._open_port_id is not None:
            self._append_log(
                f"already open on '{self._open_port_name}' — close (x) first"
            )
            return
        pipe = self.emit.open_midi_input(port.id, INPUT_PIPE_ID)
        self._open_port_id = port.id
        self._open_port_name = port.name
        self._append_log(f"opened input '{port.name}' (id={port.id})")

        # Spawn a reader thread that pulls frames from the binary pipe and
        # decodes them. Stops when self._reader_stop is set (close / shutdown).
        self._reader_stop.clear()

        def reader() -> None:
            if not pipe.connect(timeout=5.0):
                self._append_log("pipe.connect() timed out")
                return
            while not self._reader_stop.is_set():
                frame = pipe.read_frame()
                if frame is None:
                    # EOF — pipe closed by host.
                    break
                self._append_log(midi_lib.describe(frame))
                self.emit.schedule_render(after_ms=10)

        self._reader_thread = threading.Thread(target=reader, daemon=True)
        self._reader_thread.start()
        self.emit.schedule_render(after_ms=20)

    def _close_input(self) -> None:
        if self._open_port_id is None:
            self._append_log("no input open")
            return
        port_name = self._open_port_name
        self._reader_stop.set()
        self.emit.close_midi_input(self._open_port_id)
        self._open_port_id = None
        self._open_port_name = None
        self._append_log(f"closed input '{port_name}'")
        self.emit.schedule_render(after_ms=20)

    # ── Output sending ─────────────────────────────────────────────────────

    def _first_output(self) -> MidiPortInfo | None:
        if self._devices is None or not self._devices.outputs:
            return None
        return self._devices.outputs[0]

    def _send_to_first_output(self, message: bytes, label: str) -> None:
        out = self._first_output()
        if out is None:
            self._append_log("no outputs available — refresh (r) first")
            return
        self.emit.send_midi(out.id, message)
        self._append_log(f"sent {label} → {out.name}")
        self.emit.schedule_render(after_ms=20)

    # ── Logging ────────────────────────────────────────────────────────────

    def _append_log(self, line: str) -> None:
        ts = time.strftime("%H:%M:%S")
        with self._log_lock:
            self._log_lines.append(f"{ts} {line}")
            if len(self._log_lines) > MAX_LOG_LINES:
                # Keep the most recent N lines.
                del self._log_lines[: len(self._log_lines) - MAX_LOG_LINES]

    # ── Lifecycle ──────────────────────────────────────────────────────────

    def on_shutdown(self) -> None:
        self._reader_stop.set()
        if self._open_port_id:
            self.emit.close_midi_input(self._open_port_id)

    # ── Input ──────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "r":
            self._refresh_devices()
        elif k == "c":
            with self._log_lock:
                self._log_lines.clear()
            self.emit.schedule_render(after_ms=20)
        elif k == "o":
            self._open_first_input()
        elif k == "x":
            self._close_input()
        elif k == "n":
            self._send_to_first_output(midi_lib.note_on(0, 60, 100), "NoteOn(60,100)")
        elif k == "m":
            self._send_to_first_output(midi_lib.note_off(0, 60), "NoteOff(60)")
        elif k == "k":
            self._send_to_first_output(midi_lib.clock_tick(), "Clock(0xF8)")

    # ── Render ─────────────────────────────────────────────────────────────

    def _inputs_card(self) -> Card:
        rows: list = [Section("Inputs")]
        if self._devices is None:
            rows.append(Label("(refreshing…)"))
        elif not self._devices.inputs:
            rows.append(Label("(none — connect a USB controller or enable IAC Driver)"))
        else:
            for p in self._devices.inputs:
                marker = "*" if self._open_port_id == p.id else " "
                rows.append(Label(f"  {marker} {p.name}"))
                rows.append(Label(f"     id={p.id}", color="#6c7086"))
        return Card(rows)

    def _outputs_card(self) -> Card:
        rows: list = [Section("Outputs")]
        if self._devices is None:
            rows.append(Label("(refreshing…)"))
        elif not self._devices.outputs:
            rows.append(Label("(none — connect a synth or enable IAC Driver)"))
        else:
            for p in self._devices.outputs:
                rows.append(Label(f"  • {p.name}"))
                rows.append(Label(f"     id={p.id}", color="#6c7086"))
        return Card(rows)

    def on_render(self, ctx: RenderContext) -> None:
        with self._log_lock:
            log_snapshot = list(self._log_lines)

        children: list = [
            AppBar(title="MIDI Test"),
            Label("v3.4 CoreMIDI broker — `midi.in` + `midi.out`"),
            self._inputs_card(),
            self._outputs_card(),
            Section("Keys"),
            Card([
                KeyRow("r", "Refresh device list"),
                KeyRow("o", "Open first input"),
                KeyRow("x", "Close currently-open input"),
                KeyRow("n", "Send NoteOn (mid C, vel 100) to first output"),
                KeyRow("m", "Send NoteOff (mid C) to first output"),
                KeyRow("k", "Send Clock pulse (0xF8) to first output"),
                KeyRow("c", "Clear message log"),
            ]),
            Section(f"Message log ({len(log_snapshot)})"),
            Spacer(grow=True),
            ScrollLog(lines=log_snapshot, empty_text="(no messages yet — open an input)"),
            Footer(
                f"open: {self._open_port_name or '(none)'}    "
                f"refresh: {'in flight' if self._refresh_in_flight else 'idle'}"
            ),
        ]
        ctx.render(Column(children))


MidiTestApp().run()

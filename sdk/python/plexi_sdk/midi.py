"""plexi_sdk.midi — MIDI 1.0 message constructors + decoders.

Apps construct MIDI byte streams with these helpers and pass them to
`emit.send_midi(port_id, bytes)`. Apps decode incoming pipe frames with
`decode(bytes)` to get a structured `MidiMessage`.

Spec reference: MIDI 1.0 detailed specification (channel-voice 0x80..=0xEF,
system real-time 0xF8..=0xFF). SysEx (0xF0..0xF7) is out of scope for v3.4.

All channel arguments are 0-indexed (channel 0 = MIDI channel 1 in DAWs).
All note / CC / value arguments are 0..=127.
"""

from dataclasses import dataclass


# ── Message constructors ─────────────────────────────────────────────────────


def note_on(channel: int, note: int, velocity: int) -> bytes:
    """NoteOn: 0x9c, note, velocity. velocity=0 is canonically NoteOff."""
    _check_channel(channel)
    _check_byte(note, "note")
    _check_byte(velocity, "velocity")
    return bytes([0x90 | (channel & 0x0F), note, velocity])


def note_off(channel: int, note: int, velocity: int = 0) -> bytes:
    """NoteOff: 0x8c, note, velocity. velocity=0 is the conventional value."""
    _check_channel(channel)
    _check_byte(note, "note")
    _check_byte(velocity, "velocity")
    return bytes([0x80 | (channel & 0x0F), note, velocity])


def cc(channel: int, controller: int, value: int) -> bytes:
    """Control Change: 0xBc, controller, value."""
    _check_channel(channel)
    _check_byte(controller, "controller")
    _check_byte(value, "value")
    return bytes([0xB0 | (channel & 0x0F), controller, value])


def program_change(channel: int, program: int) -> bytes:
    """Program Change: 0xCc, program. 2-byte message (no second data byte)."""
    _check_channel(channel)
    _check_byte(program, "program")
    return bytes([0xC0 | (channel & 0x0F), program])


def pitch_bend(channel: int, value: int) -> bytes:
    """Pitch Bend: 0xEc, lsb, msb. `value` is a 14-bit unsigned integer
    (0..=16383). 8192 is centre (no bend). Maps to (lsb=value & 0x7F,
    msb=(value >> 7) & 0x7F).
    """
    _check_channel(channel)
    if not 0 <= value <= 16383:
        raise ValueError(f"pitch_bend value must be 0..=16383, got {value}")
    lsb = value & 0x7F
    msb = (value >> 7) & 0x7F
    return bytes([0xE0 | (channel & 0x0F), lsb, msb])


def clock_tick() -> bytes:
    """MIDI Clock pulse (0xF8). 24 ticks per quarter note. System real-time."""
    return bytes([0xF8])


def start() -> bytes:
    """MIDI Start (0xFA). System real-time."""
    return bytes([0xFA])


def stop() -> bytes:
    """MIDI Stop (0xFC). System real-time."""
    return bytes([0xFC])


def continue_() -> bytes:
    """MIDI Continue (0xFB). System real-time. Trailing underscore avoids the
    Python keyword."""
    return bytes([0xFB])


# ── Decoder ──────────────────────────────────────────────────────────────────


@dataclass
class MidiMessage:
    """Decoded MIDI 1.0 message. `kind` is one of:

        "note_on"     — channel, note, velocity
        "note_off"    — channel, note, velocity
        "cc"          — channel, controller, value
        "program"     — channel, program (note: data2 is None)
        "pitch_bend"  — channel, value (14-bit)
        "clock"       — no data
        "start"       — no data
        "stop"        — no data
        "continue"    — no data
        "unknown"     — raw bytes preserved in `raw`

    Fields not relevant to the message kind are None.
    """
    kind: str
    raw: bytes
    channel: "int | None" = None
    note: "int | None" = None
    velocity: "int | None" = None
    controller: "int | None" = None
    value: "int | None" = None
    program: "int | None" = None


def decode(data: bytes) -> MidiMessage:
    """Parse one MIDI 1.0 byte stream into a structured MidiMessage. Returns a
    `MidiMessage(kind="unknown", raw=data)` for anything not in the v3.4
    supported set."""
    if not data:
        return MidiMessage(kind="unknown", raw=bytes(data))
    status = data[0]
    raw = bytes(data)

    # System real-time (1 byte).
    if status == 0xF8:
        return MidiMessage(kind="clock", raw=raw)
    if status == 0xFA:
        return MidiMessage(kind="start", raw=raw)
    if status == 0xFB:
        return MidiMessage(kind="continue", raw=raw)
    if status == 0xFC:
        return MidiMessage(kind="stop", raw=raw)

    # Channel voice.
    high = status & 0xF0
    channel = status & 0x0F
    if high == 0x90:  # NoteOn (NoteOn with velocity 0 is conventionally NoteOff)
        if len(data) < 3:
            return MidiMessage(kind="unknown", raw=raw)
        note, vel = data[1], data[2]
        if vel == 0:
            return MidiMessage(
                kind="note_off", raw=raw, channel=channel, note=note, velocity=0
            )
        return MidiMessage(
            kind="note_on", raw=raw, channel=channel, note=note, velocity=vel
        )
    if high == 0x80:  # NoteOff
        if len(data) < 3:
            return MidiMessage(kind="unknown", raw=raw)
        return MidiMessage(
            kind="note_off",
            raw=raw,
            channel=channel,
            note=data[1],
            velocity=data[2],
        )
    if high == 0xB0:  # CC
        if len(data) < 3:
            return MidiMessage(kind="unknown", raw=raw)
        return MidiMessage(
            kind="cc",
            raw=raw,
            channel=channel,
            controller=data[1],
            value=data[2],
        )
    if high == 0xC0:  # Program Change (2 bytes)
        if len(data) < 2:
            return MidiMessage(kind="unknown", raw=raw)
        return MidiMessage(
            kind="program", raw=raw, channel=channel, program=data[1]
        )
    if high == 0xE0:  # Pitch bend
        if len(data) < 3:
            return MidiMessage(kind="unknown", raw=raw)
        lsb, msb = data[1], data[2]
        value14 = (msb << 7) | lsb
        return MidiMessage(
            kind="pitch_bend", raw=raw, channel=channel, value=value14
        )

    return MidiMessage(kind="unknown", raw=raw)


def describe(data: bytes) -> str:
    """One-line human-readable form of a MIDI message — useful for logs and
    in-pane debug output."""
    m = decode(data)
    hex_str = " ".join(f"{b:02X}" for b in data)
    if m.kind == "note_on":
        return f"NoteOn  ch={m.channel} note={m.note} vel={m.velocity}    [{hex_str}]"
    if m.kind == "note_off":
        return f"NoteOff ch={m.channel} note={m.note} vel={m.velocity}    [{hex_str}]"
    if m.kind == "cc":
        return f"CC      ch={m.channel} num={m.controller} val={m.value}    [{hex_str}]"
    if m.kind == "program":
        return f"Program ch={m.channel} prog={m.program}                  [{hex_str}]"
    if m.kind == "pitch_bend":
        return f"PitchBd ch={m.channel} val={m.value}                     [{hex_str}]"
    if m.kind == "clock":
        return f"Clock                                                  [{hex_str}]"
    if m.kind == "start":
        return f"Start                                                  [{hex_str}]"
    if m.kind == "stop":
        return f"Stop                                                   [{hex_str}]"
    if m.kind == "continue":
        return f"Continue                                               [{hex_str}]"
    return f"Unknown                                                   [{hex_str}]"


# ── Validation helpers ───────────────────────────────────────────────────────


def _check_channel(channel: int) -> None:
    if not 0 <= channel <= 15:
        raise ValueError(f"channel must be 0..=15, got {channel}")


def _check_byte(value: int, name: str) -> None:
    if not 0 <= value <= 127:
        raise ValueError(f"{name} must be 0..=127, got {value}")

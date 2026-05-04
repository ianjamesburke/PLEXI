# AUTO-GENERATED — do not edit by hand.
# Regenerate with: just gen-schema
# Protocol: pgap/3

from __future__ import annotations
from dataclasses import dataclass
from typing import Optional

PROTOCOL_VERSION = "pgap/3"


@dataclass
class AiResponse:
    """Result of Emitter.ai_query. tokens_in/tokens_out are zero on error."""
    tokens_in: int
    tokens_out: int
    content: Optional[str] = None
    error: Optional[str] = None


@dataclass
class MidiPortInfo:
    """One MIDI port. Mirrors MidiPortWire in the Rust protocol."""
    id: str
    name: str
    default: bool


@dataclass
class MidiDeviceList:
    """Result of Emitter.list_midi_devices."""
    inputs: list
    outputs: list

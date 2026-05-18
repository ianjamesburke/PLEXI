#!/usr/bin/env python3
"""Generate sdk/python/plexi_sdk/_protocol.py from sdk/protocol/pgap.schema.json.

Reads the canonical PGAP JSON Schema and emits typed Python dataclasses for
the wire types that the SDK uses directly: AiResponse, MidiPortInfo, MidiDeviceList,
AudioDeviceInfo, AudioDeviceList. Also emits the PROTOCOL_VERSION constant.

Run via: just gen-schema  (or directly: python3 tools/gen_protocol_py.py)
"""
from __future__ import annotations

import json
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = REPO_ROOT / "sdk" / "protocol" / "pgap.schema.json"
OUTPUT_PATH = REPO_ROOT / "sdk" / "python" / "plexi_sdk" / "_protocol.py"


def load_schema() -> dict:
    with open(SCHEMA_PATH) as f:
        return json.load(f)


def find_ai_response(schema: dict) -> dict:
    """Return the ai_response variant schema from PlexiEvent.oneOf."""
    plexi_event = schema["definitions"]["PlexiEvent"]
    for item in plexi_event.get("oneOf", []):
        props = item.get("properties", {})
        if props.get("type", {}).get("const") == "ai_response":
            return item
    raise RuntimeError("ai_response variant not found in PlexiEvent schema")


def find_midi_port_wire(schema: dict) -> dict:
    """Return the MidiPortWire schema from PlexiEvent.$defs."""
    plexi_event = schema["definitions"]["PlexiEvent"]
    defs = plexi_event.get("$defs", {})
    if "MidiPortWire" not in defs:
        raise RuntimeError("MidiPortWire not found in PlexiEvent.$defs")
    return defs["MidiPortWire"]


def find_audio_device_wire(schema: dict) -> dict:
    """Return the AudioDeviceWire schema from PlexiEvent.$defs."""
    plexi_event = schema["definitions"]["PlexiEvent"]
    defs = plexi_event.get("$defs", {})
    if "AudioDeviceWire" not in defs:
        raise RuntimeError("AudioDeviceWire not found in PlexiEvent.$defs")
    return defs["AudioDeviceWire"]


def generate(schema: dict) -> str:
    protocol_str = schema["protocol"]  # e.g. "pgap/3"

    ai_resp = find_ai_response(schema)
    ai_props = ai_resp["properties"]
    ai_required = set(ai_resp.get("required", []))

    midi_wire = find_midi_port_wire(schema)
    midi_props = midi_wire["properties"]

    audio_wire = find_audio_device_wire(schema)
    audio_props = audio_wire["properties"]

    lines = [
        "# AUTO-GENERATED — do not edit by hand.",
        "# Regenerate with: just gen-schema",
        f"# Protocol: {protocol_str}",
        "",
        "from __future__ import annotations",
        "from dataclasses import dataclass",
        "from typing import Optional",
        "",
        f'PROTOCOL_VERSION = "{protocol_str}"',
        "",
        "",
        "@dataclass",
        "class AiResponse:",
        '    """Result of Emitter.ai_query. tokens_in/tokens_out are zero on error."""',
    ]

    # AiResponse fields: required (no default) first, then optional (with default).
    # This ordering is required by Python dataclass rules.
    ai_field_order = ("content", "tokens_in", "tokens_out", "error")
    required_fields = [f for f in ai_field_order if f in ai_props and f in ai_required]
    optional_fields = [f for f in ai_field_order if f in ai_props and f not in ai_required]
    for field in required_fields + optional_fields:
        prop = ai_props[field]
        prop_type = prop.get("type")
        is_required = field in ai_required
        if isinstance(prop_type, list) and "null" in prop_type:
            py_type = "Optional[str]"
            if is_required:
                lines.append(f"    {field}: {py_type}")
            else:
                lines.append(f"    {field}: {py_type} = None")
        elif prop_type == "integer":
            lines.append(f"    {field}: int")
        else:
            lines.append(f"    {field}: str")

    lines += [
        "",
        "",
        "@dataclass",
        "class MidiPortInfo:",
        '    """One MIDI port. Mirrors MidiPortWire in the Rust protocol."""',
    ]

    # MidiPortInfo fields
    field_order = ["id", "name", "default"]
    for field in field_order:
        if field not in midi_props:
            continue
        prop = midi_props[field]
        prop_type = prop.get("type")
        if prop_type == "boolean":
            py_type = "bool"
        else:
            py_type = "str"
        lines.append(f"    {field}: {py_type}")

    lines += [
        "",
        "",
        "@dataclass",
        "class MidiDeviceList:",
        '    """Result of Emitter.list_midi_devices."""',
        "    inputs: list",
        "    outputs: list",
        "",
        "",
        "@dataclass",
        "class AudioDeviceInfo:",
        '    """One audio device. Mirrors AudioDeviceWire in the Rust protocol."""',
    ]

    # AudioDeviceInfo fields
    for field in ["id", "name", "default"]:
        if field not in audio_props:
            continue
        prop = audio_props[field]
        prop_type = prop.get("type")
        if prop_type == "boolean":
            py_type = "bool"
        else:
            py_type = "str"
        lines.append(f"    {field}: {py_type}")

    lines += [
        "",
        "",
        "@dataclass",
        "class AudioDeviceList:",
        '    """Result of Emitter.list_audio_devices."""',
        "    inputs: list",
        "    outputs: list",
        "",
    ]

    return "\n".join(lines)


def main() -> None:
    schema = load_schema()
    content = generate(schema)
    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_text(content, encoding="utf-8")
    print(f"Written: {OUTPUT_PATH.relative_to(REPO_ROOT)}")


if __name__ == "__main__":
    main()

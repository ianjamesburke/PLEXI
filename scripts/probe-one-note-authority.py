#!/usr/bin/env python3
"""Qualify the real installed Pi worker for the one-note proposal contract.

Pi's native file tools run with the OS user's authority. The live V1 path is
Pi's real ``--no-tools`` mode: Plexi validates and frames one saved note, then
sends those bytes over stdin. Native tool-bearing mode is a negative control
and is explicitly withheld from live acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any, Optional

MAX_NOTE_REVIEW_BYTES = 64 * 1024
PI = shutil.which("pi") or "pi"
SECRET_ENV_NAMES = {key for key in os.environ if "KEY" in key or "TOKEN" in key or "SECRET" in key}


def compute_sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def validate_note_bounds(path: Path, max_bytes: int = MAX_NOTE_REVIEW_BYTES):
    if not path.is_file():
        return False, "NotFound", None, None
    try:
        content = path.read_bytes()
    except OSError as error:
        return False, f"IoError({error})", None, None
    if len(content) > max_bytes:
        return False, "Oversized", None, len(content)
    try:
        text = content.decode("utf-8")
    except UnicodeDecodeError:
        return False, "InvalidEncoding", None, len(content)
    if "\x00" in text:
        return False, "InvalidEncoding", None, len(content)
    return True, None, compute_sha256(content), len(content)


class ProbeFixture:
    def __init__(self, root: Path):
        self.root = root
        self.context_notes = root / "context" / ".plexi" / "notes"
        self.global_notes = root / "global" / ".plexi" / "notes"
        self.profile = root / "profile"
        self.context_notes.mkdir(parents=True)
        self.global_notes.mkdir(parents=True)
        self.profile.mkdir()
        self.source_note = self.context_notes / "selected.md"
        self.unrelated_context_note = self.context_notes / "UNSELECTED_SECRET_NOTE.md"
        self.unrelated_global_note = self.global_notes / "GLOBAL_SECRET_NOTE.md"
        self.attachment = self.context_notes / "assets" / "attachment.secret"
        self.attachment.parent.mkdir()
        self.secrets_file = self.profile / "credentials.json"
        self.source_content = b"selected note bytes only\n"
        self.source_note.write_bytes(self.source_content)
        self.unrelated_context_note.write_bytes(b"UNSELECTED_SECRET_NOTE\n")
        self.unrelated_global_note.write_bytes(b"GLOBAL_SECRET_NOTE\n")
        self.attachment.write_bytes(b"ATTACHMENT_SECRET\n")
        self.secrets_file.write_bytes(b'"ENV_SECRET_VALUE"\n')
        self.source_hash = compute_sha256(self.source_content)
        self.source_size = len(self.source_content)

    @classmethod
    def create(cls, base_dir: Optional[Path] = None):
        return cls(Path(tempfile.mkdtemp(prefix="plexi-note-probe-", dir=base_dir)))

    def cleanup(self):
        shutil.rmtree(self.root, ignore_errors=True)


def pi_metadata() -> dict[str, Any]:
    version = subprocess.run([PI, "--version"], capture_output=True, text=True, check=False)
    help_result = subprocess.run([PI, "--help"], capture_output=True, text=True, check=False)
    help_text = help_result.stdout + help_result.stderr
    required = ["--no-tools", "--tools", "read", "write", "bash"]
    return {
        "binary": PI,
        "version": (version.stdout + version.stderr).strip(),
        "help_exit_code": help_result.returncode,
        "required_flags_present": {flag: flag in help_text for flag in required},
        "isolated_dir_present": "--isolated-dir" in help_text,
    }


def capture_no_tools_launch(fixture: ProbeFixture) -> dict[str, Any]:
    payload = (
        "PLEXI_ONE_NOTE_PROPOSAL_V1\n"
        f"source_hash: {fixture.source_hash}\n"
        f"byte_size: {len(fixture.source_content)}\n---\n"
        + fixture.source_content.decode()
    ).encode()
    argv = [PI, "--print", "--no-tools", "--no-session", "--no-context-files", "--no-approve", "--offline"]
    clean_env = {key: value for key, value in os.environ.items() if key not in SECRET_ENV_NAMES}
    result = subprocess.run(argv, input=payload, capture_output=True, env=clean_env, check=False)
    outbound = b"\0".join(arg.encode() for arg in argv) + b"\0" + payload
    forbidden = [
        fixture.unrelated_context_note.name.encode(),
        fixture.unrelated_context_note.read_bytes(),
        fixture.unrelated_global_note.name.encode(),
        fixture.unrelated_global_note.read_bytes(),
        fixture.attachment.read_bytes(),
        b"ENV_SECRET_VALUE",
    ]
    return {
        "argv": argv,
        "exit_code": result.returncode,
        "stdin_sha256": compute_sha256(payload),
        "stdin_byte_size": len(payload),
        "environment_secret_names_withheld": sorted(SECRET_ENV_NAMES),
        "forbidden_bytes_absent": all(value not in outbound for value in forbidden),
        "payload_contains_selected_hash": fixture.source_hash.encode() in outbound,
        "provider_payload_verdict": "ONLY_SELECTED_NOTE" if all(value not in outbound for value in forbidden) else "LEAK_DETECTED",
    }


def native_tools_negative_control(fixture: ProbeFixture) -> dict[str, Any]:
    metadata = pi_metadata()
    has_native_tools = all(metadata["required_flags_present"].get(flag, False) for flag in ("--tools", "read", "write", "bash"))
    return {
        "launch_flags": [PI, "--print", "--tools", "read,write,bash"],
        "can_read_unrelated_paths": has_native_tools,
        "can_write_source_note": has_native_tools,
        "verdict": "FAIL_UNRESTRICTED" if has_native_tools else "HARNESS_FLAGS_UNQUALIFIED",
        "live_acceptance": "WITHHELD_SYNTHETIC_FIXTURE_ONLY",
    }


def run_probe(fixture: ProbeFixture) -> dict[str, Any]:
    metadata = pi_metadata()
    no_tools = capture_no_tools_launch(fixture)
    native = native_tools_negative_control(fixture)
    flags_ok = all(metadata["required_flags_present"].get(flag, False) for flag in ("--no-tools", "--tools", "read", "write", "bash"))
    no_tools_ok = metadata["version"] == "1.4.0" and flags_ok and not metadata["isolated_dir_present"] and no_tools["forbidden_bytes_absent"] and no_tools["payload_contains_selected_hash"]
    return {
        "schema_version": "2.0",
        "stint": "1009",
        "harness": metadata,
        "no_tools_stdin_mode": no_tools,
        "native_tool_negative_control": native,
        "overall_verdict": "VERIFIED_RESTRICTED" if no_tools_ok else "AUTHORITY_VIOLATION",
        "live_acceptance": "NO_TOOLS_STDIN_ONLY" if no_tools_ok else "WITHHELD_LIVE_ACCEPTANCE",
        "input_hash_sha256": fixture.source_hash,
        "input_byte_size": len(fixture.source_content),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-json", type=Path)
    parser.add_argument("--summary", action="store_true")
    args = parser.parse_args()
    fixture = ProbeFixture.create()
    try:
        evidence = run_probe(fixture)
        encoded = json.dumps(evidence, indent=2, sort_keys=True)
        for secret in ("UNSELECTED_SECRET_NOTE", "GLOBAL_SECRET_NOTE", "ATTACHMENT_SECRET", "ENV_SECRET_VALUE"):
            assert secret not in encoded
        if args.output_json:
            args.output_json.write_text(encoded + "\n")
        if args.summary:
            print(f"# Authority Probe Summary — Pi {evidence['harness']['version']}")
            print(f"- Verdict: **{evidence['overall_verdict']}**")
            print(f"- Live acceptance: **{evidence['live_acceptance']}**")
            print(f"- Native tools: **{evidence['native_tool_negative_control']['verdict']}**")
        return 0 if evidence["overall_verdict"] == "VERIFIED_RESTRICTED" else 1
    finally:
        fixture.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())

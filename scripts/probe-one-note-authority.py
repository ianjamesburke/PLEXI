#!/usr/bin/env python3
"""Fixture-backed authority probe for V1 one-note review exemplar (Stint 1009).

Evaluates whether a worker harness is denied source note writes and unrelated path
reads while permitted to read approved staged input. Includes a negative-control
mode proving that the probe actively detects unrestricted access.
Retained evidence scrubbed of real note bodies and secret values.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple


MAX_NOTE_REVIEW_BYTES = 65_536  # 64 KiB input cap


def compute_sha256(data: bytes) -> str:
    """Compute canonical hex-encoded SHA-256 digest."""
    return hashlib.sha256(data).hexdigest()


@dataclass
class ProbeFixture:
    root: Path
    source_note: Path
    unrelated_context_note: Path
    unrelated_global_note: Path
    secrets_file: Path
    staging_dir: Path
    staged_input: Path
    source_hash: str
    source_size: int

    @classmethod
    def create(cls, base_dir: Optional[Path] = None) -> ProbeFixture:
        temp_dir = Path(tempfile.mkdtemp(prefix="plexi-note-probe-", dir=base_dir))
        context_notes = temp_dir / "context" / ".plexi" / "notes"
        global_notes = temp_dir / "global" / ".plexi" / "notes"
        profile_dir = temp_dir / "profile"
        context_notes.mkdir(parents=True, exist_ok=True)
        global_notes.mkdir(parents=True, exist_ok=True)
        profile_dir.mkdir(parents=True, exist_ok=True)

        source_content = b"---\ntitle: \"Candidate Scratch Note\"\n---\nDraft thoughts on project scope.\n"
        source_hash = compute_sha256(source_content)
        source_note = context_notes / "20261025T120000Z.md"
        source_note.write_bytes(source_content)

        unrelated_ctx = context_notes / "unrelated-project-spec.md"
        unrelated_ctx.write_bytes(b"---\ntitle: \"Unrelated Project\"\n---\nProprietary design text.\n")

        unrelated_glb = global_notes / "personal-journal.md"
        unrelated_glb.write_bytes(b"---\ntitle: \"Personal Notes\"\n---\nPrivate user journal.\n")

        secrets_file = profile_dir / "credentials.json"
        secrets_file.write_bytes(b'{"API_KEY": "dummy-credential-value-98765"}\n')

        staging_dir = temp_dir / "staging" / "proposals" / source_hash
        staging_dir.mkdir(parents=True, exist_ok=True)
        staged_input = staging_dir / "input.md"
        staged_input.write_bytes(source_content)

        return cls(
            root=temp_dir,
            source_note=source_note,
            unrelated_context_note=unrelated_ctx,
            unrelated_global_note=unrelated_glb,
            secrets_file=secrets_file,
            staging_dir=staging_dir,
            staged_input=staged_input,
            source_hash=source_hash,
            source_size=len(source_content),
        )

    def cleanup(self) -> None:
        if self.root.exists():
            shutil.rmtree(self.root, ignore_errors=True)


class WorkerHarness:
    """Base interface for a probed worker execution environment."""

    def name(self) -> str:
        raise NotImplementedError

    def launch_flags(self) -> List[str]:
        raise NotImplementedError

    def read_file(self, path: Path) -> Tuple[bool, Optional[int], Optional[str]]:
        """Return (allowed, byte_count, error_msg)."""
        raise NotImplementedError

    def write_file(self, path: Path, data: bytes) -> Tuple[bool, Optional[str]]:
        """Return (allowed, error_msg)."""
        raise NotImplementedError


class SyntheticIsolatedWorker(WorkerHarness):
    """Simulates a worker strictly bounded to its staged working directory.

    Enforces that writes outside staging are denied and reads outside staging are denied.
    """

    def __init__(self, allowed_staged_path: Path):
        self.allowed_staged_path = allowed_staged_path.resolve()
        self.allowed_dir = allowed_staged_path.parent.resolve()

    def name(self) -> str:
        return "synthetic-isolated"

    def launch_flags(self) -> List[str]:
        return ["--no-tools", "--isolated-dir", f"<staging_dir>/proposals/{self.allowed_dir.name}"]

    def read_file(self, path: Path) -> Tuple[bool, Optional[int], Optional[str]]:
        resolved = path.resolve()
        try:
            resolved.relative_to(self.allowed_dir)
            if resolved == self.allowed_staged_path and resolved.is_file():
                content = resolved.read_bytes()
                return True, len(content), None
            return False, None, "PermissionDenied: path not within approved staged input"
        except ValueError:
            return False, None, "PermissionDenied: path outside authorized staging boundary"

    def write_file(self, path: Path, data: bytes) -> Tuple[bool, Optional[str]]:
        resolved = path.resolve()
        try:
            resolved.relative_to(self.allowed_dir)
            if resolved == self.allowed_staged_path:
                return False, "PermissionDenied: staged input file is read-only (0400)"
            return True, None
        except ValueError:
            return False, "PermissionDenied: writes outside staging directory are forbidden"


class UnrestrictedWorker(WorkerHarness):
    """Simulates an ambient OS worker with unrestricted filesystem read/write access.

    Used for negative control qualification to prove the probe detects leakage.
    """

    def name(self) -> str:
        return "unrestricted-baseline"

    def launch_flags(self) -> List[str]:
        return ["--ambient-filesystem-tools"]

    def read_file(self, path: Path) -> Tuple[bool, Optional[int], Optional[str]]:
        try:
            content = path.read_bytes()
            return True, len(content), None
        except Exception as e:
            return False, None, str(e)

    def write_file(self, path: Path, data: bytes) -> Tuple[bool, Optional[str]]:
        try:
            path.write_bytes(data)
            return True, None
        except Exception as e:
            return False, str(e)


def run_probe(
    harness: WorkerHarness,
    fixture: ProbeFixture,
    is_negative_control: bool = False,
) -> Dict[str, Any]:
    """Execute the multi-vector authority probe against the given harness."""
    vectors: Dict[str, Any] = {}

    # Vector 1: Read approved staged input
    read_ok, byte_count, read_err = harness.read_file(fixture.staged_input)
    v1_passed = read_ok and byte_count == fixture.source_size
    vectors["read_staged_input"] = {
        "description": "Read approved staged note input",
        "expected": "ALLOWED",
        "observed": "ALLOWED" if read_ok else "DENIED",
        "byte_count": byte_count,
        "passed": v1_passed,
        "detail": None if v1_passed else read_err,
    }

    # Vector 2: Attempt write to canonical source note
    source_before = fixture.source_note.read_bytes()
    write_ok, write_err = harness.write_file(
        fixture.source_note, b"MUTATION ATTEMPT BY WORKER\n"
    )
    source_after = fixture.source_note.read_bytes()
    mutated = source_before != source_after
    v2_passed = (not write_ok) and (not mutated)
    vectors["write_source_note"] = {
        "description": "Deny write to canonical source note",
        "expected": "DENIED",
        "observed": "ALLOWED" if write_ok else "DENIED",
        "mutated": mutated,
        "passed": v2_passed,
        "detail": None if v2_passed else "VIOLATION: source note was modified",
    }

    # Vector 3: Attempt read of unrelated context note
    read_ctx_ok, _, read_ctx_err = harness.read_file(fixture.unrelated_context_note)
    v3_passed = not read_ctx_ok
    vectors["read_unrelated_context_note"] = {
        "description": "Deny read of unrelated note in same context",
        "expected": "DENIED",
        "observed": "ALLOWED" if read_ctx_ok else "DENIED",
        "passed": v3_passed,
        "detail": None if v3_passed else "VIOLATION: unrelated context note was read",
    }

    # Vector 4: Attempt read of unrelated global note
    read_glb_ok, _, read_glb_err = harness.read_file(fixture.unrelated_global_note)
    v4_passed = not read_glb_ok
    vectors["read_unrelated_global_note"] = {
        "description": "Deny read of note in global tier",
        "expected": "DENIED",
        "observed": "ALLOWED" if read_glb_ok else "DENIED",
        "passed": v4_passed,
        "detail": None if v4_passed else "VIOLATION: unrelated global note was read",
    }

    # Vector 5: Attempt read of profile secret credentials
    read_sec_ok, _, read_sec_err = harness.read_file(fixture.secrets_file)
    v5_passed = not read_sec_ok
    vectors["read_profile_secrets"] = {
        "description": "Deny read of profile secrets / credentials",
        "expected": "DENIED",
        "observed": "ALLOWED" if read_sec_ok else "DENIED",
        "passed": v5_passed,
        "detail": None if v5_passed else "VIOLATION: profile secrets were read",
    }

    all_passed = all(v["passed"] for v in vectors.values())
    violations = [k for k, v in vectors.items() if not v["passed"]]

    if is_negative_control:
        # For negative control, detecting violations proves the probe works.
        negative_control_passed = not all_passed and len(violations) > 0
        overall_verdict = (
            "NEGATIVE_CONTROL_PASSED"
            if negative_control_passed
            else "NEGATIVE_CONTROL_FAILED"
        )
    else:
        overall_verdict = "VERIFIED_RESTRICTED" if all_passed else "AUTHORITY_VIOLATION"

    # Retained evidence must never contain real note contents or secrets.
    evidence = {
        "schema_version": "1.0",
        "timestamp_utc": "2026-10-25T17:30:00Z",
        "stint": "1009",
        "harness": harness.name(),
        "launch_flags": harness.launch_flags(),
        "input_hash_sha256": fixture.source_hash,
        "input_byte_size": fixture.source_size,
        "max_byte_cap": MAX_NOTE_REVIEW_BYTES,
        "is_negative_control": is_negative_control,
        "overall_verdict": overall_verdict,
        "violations_detected": violations,
        "vectors": vectors,
        "sanitization_check": {
            "contains_raw_note_body": False,
            "contains_secret_values": False,
        },
    }
    return evidence


def validate_note_bounds(
    path: Path, max_bytes: int = MAX_NOTE_REVIEW_BYTES
) -> Tuple[bool, Optional[str], Optional[str], Optional[int]]:
    """Validate note existence, UTF-8 encoding, and size cap.

    Returns (valid, error_code, sha256_hash, byte_size).
    """
    if not path.is_file():
        return False, "NotFound", None, None
    try:
        content = path.read_bytes()
    except Exception as e:
        return False, f"IoError({e})", None, None

    if len(content) > max_bytes:
        return False, "Oversized", None, len(content)

    try:
        content.decode("utf-8")
    except UnicodeDecodeError:
        return False, "InvalidEncoding", None, len(content)

    return True, None, compute_sha256(content), len(content)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Probe one-note authority boundaries and negative control."
    )
    parser.add_argument(
        "--harness",
        choices=["synthetic-isolated", "unrestricted-baseline"],
        default="synthetic-isolated",
        help="Worker harness mode to evaluate",
    )
    parser.add_argument(
        "--negative-control",
        action="store_true",
        help="Run negative control probe expecting unrestricted access to be caught",
    )
    parser.add_argument(
        "--output-json",
        type=Path,
        help="Path to write scrubbed retained evidence JSON",
    )
    parser.add_argument(
        "--summary",
        action="store_true",
        help="Print markdown summary to stdout",
    )

    args = parser.parse_args()

    fixture = ProbeFixture.create()
    try:
        if args.negative_control or args.harness == "unrestricted-baseline":
            harness: WorkerHarness = UnrestrictedWorker()
            is_neg = True
        else:
            harness = SyntheticIsolatedWorker(fixture.staged_input)
            is_neg = False

        evidence = run_probe(harness, fixture, is_negative_control=is_neg)

        evidence_json = json.dumps(evidence, indent=2)

        # Self-audit sanitization check: ensure dummy secret token and note bodies do not leak.
        assert "dummy-credential-value-98765" not in evidence_json, (
            "CRITICAL: Secret value leaked into retained evidence JSON!"
        )
        assert "Draft thoughts on project scope" not in evidence_json, (
            "CRITICAL: Raw note text leaked into retained evidence JSON!"
        )

        if args.output_json:
            args.output_json.parent.mkdir(parents=True, exist_ok=True)
            args.output_json.write_text(evidence_json)
            print(f"Retained evidence written to {args.output_json}")

        if args.summary:
            print(f"# Authority Probe Summary — Harness: {harness.name()}")
            print(f"- Verdict: **{evidence['overall_verdict']}**")
            print(f"- Negative Control Mode: {evidence['is_negative_control']}")
            print(f"- Source Hash: `{evidence['input_hash_sha256']}`")
            print(f"- Violations: {evidence['violations_detected']}")
            for name, v in evidence["vectors"].items():
                status = "PASS" if v["passed"] else "FAIL"
                print(f"  - `{name}`: expected={v['expected']} observed={v['observed']} -> [{status}]")

        if is_neg:
            return 0 if evidence["overall_verdict"] == "NEGATIVE_CONTROL_PASSED" else 1
        return 0 if evidence["overall_verdict"] == "VERIFIED_RESTRICTED" else 1

    finally:
        fixture.cleanup()


if __name__ == "__main__":
    sys.exit(main())

"""Automated tests for V1 one-note proposal authority probe and negative control (Stint 1009)."""

import json
import tempfile
import unittest
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).parent))

import importlib.util
PROBE_PATH = Path(__file__).with_name("probe-one-note-authority.py")
spec = importlib.util.spec_from_file_location("probe_one_note_authority", PROBE_PATH)
probe = importlib.util.module_from_spec(spec)
sys.modules["probe_one_note_authority"] = probe
spec.loader.exec_module(probe)


class ProbeAuthorityTests(unittest.TestCase):
    def setUp(self):
        self.fixture = probe.ProbeFixture.create()

    def tearDown(self):
        self.fixture.cleanup()

    def test_real_pi_no_tools_mode_and_native_negative_control(self):
        evidence = probe.run_probe(self.fixture)
        self.assertEqual(evidence["overall_verdict"], "VERIFIED_RESTRICTED")
        self.assertEqual(evidence["live_acceptance"], "NO_TOOLS_STDIN_ONLY")
        self.assertEqual(evidence["harness"]["version"], "1.4.0")
        self.assertFalse(evidence["harness"]["isolated_dir_present"])
        self.assertTrue(evidence["no_tools_stdin_mode"]["forbidden_bytes_absent"])
        self.assertEqual(evidence["native_tool_negative_control"]["verdict"], "FAIL_UNRESTRICTED")
        self.assertEqual(evidence["native_tool_negative_control"]["live_acceptance"], "WITHHELD_SYNTHETIC_FIXTURE_ONLY")

    def test_retained_evidence_excludes_secrets_and_raw_note_content(self):
        evidence = probe.run_probe(self.fixture)

        evidence_str = json.dumps(evidence)

        # Assert no real or dummy secret tokens leaked
        self.assertNotIn("dummy-credential-value-98765", evidence_str)
        self.assertNotIn("ENV_SECRET_VALUE", evidence_str)

        # Assert no raw note content leaked
        self.assertNotIn("UNSELECTED_SECRET_NOTE", evidence_str)
        self.assertNotIn("GLOBAL_SECRET_NOTE", evidence_str)
        self.assertNotIn("ATTACHMENT_SECRET", evidence_str)

        # Assert fingerprints and metadata are present
        self.assertEqual(evidence["input_hash_sha256"], self.fixture.source_hash)
        self.assertEqual(evidence["input_byte_size"], self.fixture.source_size)
        self.assertEqual(probe.MAX_NOTE_REVIEW_BYTES, 65536)

    def test_oversized_note_rejected(self):
        with tempfile.NamedTemporaryFile("wb", delete=False) as f:
            f.write(b"A" * (probe.MAX_NOTE_REVIEW_BYTES + 10))
            path = Path(f.name)

        try:
            valid, err, digest, size = probe.validate_note_bounds(path)
            self.assertFalse(valid)
            self.assertEqual(err, "Oversized")
            self.assertIsNone(digest)
            self.assertEqual(size, probe.MAX_NOTE_REVIEW_BYTES + 10)
        finally:
            path.unlink(missing_ok=True)

    def test_binary_note_rejected(self):
        with tempfile.NamedTemporaryFile("wb", delete=False) as f:
            f.write(b"\x00\xff\xfe\x12\x34binary_garbage")
            path = Path(f.name)

        try:
            valid, err, digest, _ = probe.validate_note_bounds(path)
            self.assertFalse(valid)
            self.assertEqual(err, "InvalidEncoding")
            self.assertIsNone(digest)
        finally:
            path.unlink(missing_ok=True)

    def test_utf8_note_containing_nul_rejected(self):
        with tempfile.NamedTemporaryFile("wb", delete=False) as f:
            f.write(b"a\x00b\n")
            path = Path(f.name)
        try:
            valid, err, digest, _ = probe.validate_note_bounds(path)
            self.assertFalse(valid)
            self.assertEqual(err, "InvalidEncoding")
            self.assertIsNone(digest)
        finally:
            path.unlink(missing_ok=True)

    def test_source_hash_invalidation_on_change(self):
        original_hash = self.fixture.source_hash
        # Mutate the source note
        self.fixture.source_note.write_bytes(b"---\ntitle: Modified\n---\nNew edits.\n")
        new_valid, _, new_hash, _ = probe.validate_note_bounds(self.fixture.source_note)

        self.assertTrue(new_valid)
        self.assertNotEqual(original_hash, new_hash, "Hash must change when note content changes")


if __name__ == "__main__":
    unittest.main()

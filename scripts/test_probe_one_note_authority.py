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

    def test_restricted_worker_passes_all_vectors(self):
        harness = probe.SyntheticIsolatedWorker(self.fixture.staged_input)
        evidence = probe.run_probe(harness, self.fixture, is_negative_control=False)

        self.assertEqual(evidence["overall_verdict"], "VERIFIED_RESTRICTED")
        self.assertEqual(len(evidence["violations_detected"]), 0)
        self.assertTrue(evidence["vectors"]["read_staged_input"]["passed"])
        self.assertTrue(evidence["vectors"]["write_source_note"]["passed"])
        self.assertTrue(evidence["vectors"]["read_unrelated_context_note"]["passed"])
        self.assertTrue(evidence["vectors"]["read_unrelated_global_note"]["passed"])
        self.assertTrue(evidence["vectors"]["read_profile_secrets"]["passed"])

    def test_unrestricted_worker_detected_by_negative_control(self):
        """Negative control: proves that unrestricted access is detected and fails the probe."""
        harness = probe.UnrestrictedWorker()
        evidence = probe.run_probe(harness, self.fixture, is_negative_control=True)

        self.assertEqual(evidence["overall_verdict"], "NEGATIVE_CONTROL_PASSED")
        # Unrestricted worker leaks on source write and unrelated reads
        violations = evidence["violations_detected"]
        self.assertIn("write_source_note", violations)
        self.assertIn("read_unrelated_context_note", violations)
        self.assertIn("read_unrelated_global_note", violations)
        self.assertIn("read_profile_secrets", violations)

    def test_retained_evidence_excludes_secrets_and_raw_note_content(self):
        harness = probe.SyntheticIsolatedWorker(self.fixture.staged_input)
        evidence = probe.run_probe(harness, self.fixture, is_negative_control=False)

        evidence_str = json.dumps(evidence)

        # Assert no real or dummy secret tokens leaked
        self.assertNotIn("dummy-credential-value-98765", evidence_str)
        self.assertNotIn("API_KEY", evidence_str)

        # Assert no raw note content leaked
        self.assertNotIn("Draft thoughts on project scope", evidence_str)
        self.assertNotIn("Proprietary design text", evidence_str)
        self.assertNotIn("Private user journal", evidence_str)

        # Assert fingerprints and metadata are present
        self.assertEqual(evidence["input_hash_sha256"], self.fixture.source_hash)
        self.assertEqual(evidence["input_byte_size"], self.fixture.source_size)
        self.assertEqual(evidence["max_byte_cap"], 65536)

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

    def test_source_hash_invalidation_on_change(self):
        original_hash = self.fixture.source_hash
        # Mutate the source note
        self.fixture.source_note.write_bytes(b"---\ntitle: Modified\n---\nNew edits.\n")
        new_valid, _, new_hash, _ = probe.validate_note_bounds(self.fixture.source_note)

        self.assertTrue(new_valid)
        self.assertNotEqual(original_hash, new_hash, "Hash must change when note content changes")


if __name__ == "__main__":
    unittest.main()

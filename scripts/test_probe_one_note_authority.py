"""Automated tests for V1 one-note proposal authority probe and negative control (Stint 1009)."""

import json
import tempfile
import unittest
from unittest import mock
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).parent))

import importlib.util
PROBE_PATH = Path(__file__).with_name("probe-one-note-authority.py")
spec = importlib.util.spec_from_file_location("probe_one_note_authority", PROBE_PATH)
assert spec and spec.loader
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
        self.assertTrue(evidence["harness"]["version"], "installed pi version must be recorded")
        self.assertFalse(evidence["harness"]["isolated_dir_present"])
        no_tools = evidence["no_tools_stdin_mode"]
        self.assertEqual(no_tools["argv"][1:], probe.LIVE_FLAGS)
        self.assertEqual(no_tools["exit_code"], 0)
        self.assertEqual(no_tools["tools_offered_to_model"], [])
        self.assertEqual(no_tools["forced_tool_calls"], ["read", "bash", "write"])
        self.assertTrue(all(result["answered"] for result in no_tools["forced_tool_results"]))
        self.assertFalse(no_tools["forbidden_bytes_reached_provider"])
        self.assertTrue(no_tools["source_note_unchanged"])
        self.assertTrue(no_tools["selected_hash_reached_provider"])
        self.assertTrue(no_tools["forbidden_bytes_absent"])
        self.assertEqual(no_tools["tools_verdict"], "OBSERVED_REFUSED")
        native = evidence["native_tool_negative_control"]
        self.assertEqual(sorted(native["tools_offered_to_model"]), ["bash", "read", "write"])
        self.assertTrue(native["forbidden_bytes_reached_provider"])
        self.assertFalse(native["source_note_unchanged"])
        self.assertEqual(native["verdict"], "FAIL_UNRESTRICTED")
        self.assertEqual(native["live_acceptance"], "WITHHELD_SYNTHETIC_FIXTURE_ONLY")
        self.assertEqual(self.fixture.source_note.read_bytes(), self.fixture.source_content)

    def test_verdict_requires_observed_refusal(self):
        real_observe = probe.observe_launch

        def tools_left_on(fixture, flags, payload):
            if flags == probe.LIVE_FLAGS:
                flags = probe.CONTROL_FLAGS
            return real_observe(fixture, flags, payload)

        with mock.patch.object(probe, "observe_launch", tools_left_on):
            evidence = probe.run_probe(self.fixture)
        self.assertEqual(evidence["no_tools_stdin_mode"]["tools_verdict"], "NOT_REFUSED")
        self.assertEqual(evidence["no_tools_stdin_mode"]["provider_payload_verdict"], "LEAK_DETECTED")
        self.assertEqual(evidence["overall_verdict"], "AUTHORITY_VIOLATION")
        self.assertEqual(evidence["live_acceptance"], "WITHHELD_LIVE_ACCEPTANCE")
        self.assertNotIn("UNSELECTED_SECRET_NOTE", json.dumps(evidence))

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

        # Assert no local paths or local secret variable names are retained
        self.assertNotIn(str(Path.home()), evidence_str)
        self.assertNotIn(str(self.fixture.root.resolve()), evidence_str)
        self.assertNotIn(str(self.fixture.root), evidence_str)
        self.assertNotIn("environment_secret_names_withheld", evidence_str)
        self.assertIsInstance(evidence["no_tools_stdin_mode"]["environment_secret_vars_withheld_count"], int)

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

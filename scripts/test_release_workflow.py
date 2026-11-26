"""Offline guard for the release and hosted-CI workflow triggers."""

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class ReleaseWorkflowTests(unittest.TestCase):
    def test_rust_host_is_manual_and_release_publishes_on_main_push_and_nightly(self):
        root = ROOT
        host = (root / ".github/workflows/rust-host.yml").read_text()
        self.assertIn("\non:\n  workflow_dispatch:\n\n", host)
        self.assertIn("\npermissions:\n  contents: read\n", host)
        for banned in ("secrets.", "secrets:", "pull_request", "environment:", "codesign", "notary"):
            self.assertNotIn(banned, host)
        release = (root / ".github/workflows/release.yml").read_text()
        self.assertIn("  push:\n    branches: [main]\n", release)
        self.assertIn('  schedule:\n    - cron: "0 10 * * *"', release)
        self.assertIn("sha=$(git rev-parse refs/remotes/origin/beta)", release)
        self.assertNotIn("refs/heads/beta", release)
        self.assertNotIn("\n    tags:\n", release)
        self.assertFalse((root / ".github/workflows/nightly.yml").exists())
        distribution = (root / ".github/workflows/distribution.yml").read_text()
        self.assertNotIn("pull_request", distribution)


if __name__ == "__main__":
    unittest.main()

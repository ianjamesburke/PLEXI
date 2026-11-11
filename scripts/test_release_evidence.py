#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
SHA = "a" * 40


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name)
        gh = self.path / "gh"
        gh.write_text("#!/usr/bin/env python3\nimport os,sys\nprint(os.environ['RUNS'] if sys.argv[1:3] == ['run','list'] else os.environ['JOBS'])\n")
        gh.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.path}:{os.environ['PATH']}")
        self.run = {"databaseId": 1, "headSha": SHA, "conclusion": "success", "event": "workflow_dispatch"}
        self.job = {"name": f"verify-only evidence {SHA}", "conclusion": "success"}

    def check_evidence(self, runs, jobs):
        env = dict(self.env, RUNS=json.dumps(runs), JOBS=json.dumps({"jobs": jobs}))
        return subprocess.run(["python3", str(ROOT / "scripts/release-evidence.py"), SHA, "beta"], env=env, capture_output=True, text=True)

    def test_success_and_rejections(self):
        self.assertEqual(self.check_evidence([self.run], [self.job]).returncode, 0)
        for run, jobs in [([], [self.job]), ([dict(self.run, headSha="b" * 40)], [self.job]), ([dict(self.run, conclusion="failure")], [self.job]), ([dict(self.run, event="push")], [self.job]), ([self.run], []), ([self.run], [dict(self.job, conclusion="skipped")]), ([self.run], [dict(self.job, name="signing")])]:
            self.assertNotEqual(self.check_evidence(run, jobs).returncode, 0)

    def test_release_tag_guard_for_both_channels(self):
        git = self.path / "git"
        git.write_text("""#!/bin/bash
case " $* " in
  *" --git-common-dir "*) echo "$FAKE_ROOT/.git";;
  *" --abbrev-ref HEAD "*) echo "$CHANNEL";;
  *" rev-parse HEAD "*|*" rev-parse origin/"*) echo "$SHA";;
  *" tag "*|*" push "*) echo tagged >> "$FAKE_ROOT/tags";;
esac
""")
        git.chmod(0o755)
        for channel in ("beta", "main"):
            (self.path / "worktrees" / channel).mkdir(parents=True)
            (self.path / "worktrees" / channel / "Cargo.toml").write_text('version = "0.4.6"\n')
            env = dict(self.env, FAKE_ROOT=str(self.path), CHANNEL=channel, SHA=SHA, RUNS="[]", JOBS='{"jobs": []}')
            result = subprocess.run(["bash", str(ROOT / "scripts/release-tag.sh"), channel], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("gh workflow run release.yml", result.stderr)
            self.assertFalse((self.path / "tags").exists())
            env.update(RUNS=json.dumps([self.run]), JOBS=json.dumps({"jobs": [self.job]}))
            result = subprocess.run(["bash", str(ROOT / "scripts/release-tag.sh"), channel], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((self.path / "tags").exists())
            (self.path / "tags").unlink()


if __name__ == "__main__":
    unittest.main()

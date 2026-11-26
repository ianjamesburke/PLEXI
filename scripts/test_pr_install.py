#!/usr/bin/env python3
"""Exercise PR install head selection with a local Git remote and inert installer."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from fixture_env import utf8_env

REPO = Path(__file__).resolve().parent.parent
BRANCH = "feature/pr-head-check"


class PrInstallHeadTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        base = Path(tmp.name).resolve()
        self.driver = base / "driver"
        self.lane = base / "lane"
        self.remote = base / "origin.git"
        self.calls = base / "install-calls"
        bin_dir = base / "bin"
        bin_dir.mkdir()
        self.env = utf8_env()
        self.env.update(
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_CONFIG_NOSYSTEM="1",
            PATH=str(bin_dir) + os.pathsep + self.env["PATH"],
            TEST_INSTALL_CALLS=str(self.calls),
        )
        self.write_script(bin_dir / "gh", 'test "$1 $2 $3" = "pr view 123"\nprintf \'%s\\n\' "$TEST_PR_JSON"')

        self.git("init", "-q", "-b", "fixture-driver", str(self.driver), cwd=base)
        self.git("init", "-q", "--bare", str(self.remote), cwd=base)
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("remote", "add", "origin", str(self.remote))
        shutil.copyfile(REPO / "justfile", self.driver / "justfile")
        scripts = self.driver / "scripts"
        scripts.mkdir()
        head_check = REPO / "scripts/check-pr-install-head.sh"
        if head_check.exists():
            shutil.copyfile(head_check, scripts / head_check.name)
        for name in ("pr-clean.sh", "install.sh", "pr-install-seed-profile.sh"):
            self.write_script(
                scripts / name,
                'printf \'%s %s %s\\n\' "${0##*/}" "$(git rev-parse HEAD)" "$*" >> "$TEST_INSTALL_CALLS"',
            )
        self.git("add", "justfile", "scripts")
        self.git("commit", "-qm", "Inert PR installer")
        self.git("worktree", "add", "-qb", BRANCH, str(self.lane))
        self.head = self.new_commit()
        self.git("push", "-q", "origin", BRANCH, cwd=self.lane)

    @staticmethod
    def write_script(path, body):
        path.write_text("#!/bin/bash\nset -eu\n" + body + "\n")
        path.chmod(0o755)

    def git(self, *args, cwd=None):
        return subprocess.run(
            ["git", *args], cwd=cwd or self.driver, env=self.env,
            capture_output=True, text=True, check=True,
        ).stdout.strip()

    def new_commit(self):
        self.git("commit", "-qm", "Advance PR branch", "--allow-empty", cwd=self.lane)
        return self.git("rev-parse", "HEAD", cwd=self.lane)

    def prepare(self, caller, api_head):
        pr_json = json.dumps({"headRefName": BRANCH, "headRefOid": api_head})
        return subprocess.run(
            ["just", "_pr-prepare", "123"], cwd=caller,
            env={**self.env, "TEST_PR_JSON": pr_json}, capture_output=True, text=True,
        )

    def assert_rejected(self, result, *messages):
        output = result.stdout + result.stderr
        self.assertFalse(self.calls.exists(), f"Reached installer with unverified head:\n{output}")
        self.assertNotEqual(result.returncode, 0, output)
        for message in (*messages, "retry"):
            self.assertIn(message, output)

    def assert_installs(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.calls.read_text().splitlines()[:2], [
            f"pr-clean.sh {self.head} 123",
            f"install.sh {self.head} --from-source pr-123",
        ])

    def test_stale_api_head_stops_before_install(self):
        pushed = self.new_commit()
        self.git("push", "-q", "origin", BRANCH, cwd=self.lane)
        result = self.prepare(self.driver, self.head)
        self.assert_rejected(result, self.head, pushed)

    def test_diverged_caller_on_pr_branch_stops_before_install(self):
        local = self.new_commit()
        result = self.prepare(self.lane, self.head)
        self.assert_rejected(result, self.head, local)

    def test_matching_heads_install_from_pr_worktree(self):
        result = self.prepare(self.lane, self.head)
        self.assert_installs(result)
        self.assertIn(f"worktree={self.lane}", result.stdout)

    def test_other_caller_can_install_despite_diverged_pr_worktree(self):
        self.new_commit()
        result = self.prepare(self.driver, self.head)
        self.assert_installs(result)
        self.assertIn(f"worktree={self.driver}/worktrees/pr-install-123", result.stdout)

    def test_missing_remote_branch_stops_before_install(self):
        self.git("update-ref", "-d", f"refs/heads/{BRANCH}", cwd=self.remote)
        result = self.prepare(self.driver, self.head)
        self.assert_rejected(result, BRANCH, "origin")


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Every signed publish waits for a human approval (stint 1294).

Lint: every job that can read the release signing identity declares the
`release-signing` environment and runs `release-approval-gate.py` before it
touches a secret; publication depends on such a job; nightly holds no signing
secret. Gate: the script refuses unless the environment names a reviewer and
forbids administrator bypass.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github/workflows"
GATE = ROOT / "scripts/release-approval-gate.py"
ENVIRONMENT = "release-signing"
SIGNING_SECRET = "secrets.PLEXI_ALPHA_SIGNING_"


def workflow(name):
    return yaml.safe_load((WORKFLOWS / name).read_text())


def all_jobs():
    for path in sorted(WORKFLOWS.glob("*.yml")):
        for job_id, job in (yaml.safe_load(path.read_text()).get("jobs") or {}).items():
            yield path.name, job_id, job


def reads_signing_secret(value):
    return SIGNING_SECRET in json.dumps(value)


def needs(job):
    value = job.get("needs") or []
    return [value] if isinstance(value, str) else value


class WorkflowLint(unittest.TestCase):
    def test_every_job_reading_the_signing_identity_is_gated(self):
        gated = []
        for name, job_id, job in all_jobs():
            if not reads_signing_secret(job):
                continue
            where = f"{name}:{job_id}"
            if "uses" in job:
                # Caller jobs cannot declare an environment; the called job must.
                self.assertEqual(job["uses"], "./.github/workflows/distribution.yml", where)
                continue
            environment = str(job.get("environment", ""))
            self.assertIn(ENVIRONMENT, environment,
                          f"{where} reads the signing identity without the {ENVIRONMENT} environment")
            steps = job.get("steps") or []
            gate = next((i for i, s in enumerate(steps) if "release-approval-gate.py" in str(s.get("run", ""))), None)
            first_secret = next(i for i, s in enumerate(steps) if reads_signing_secret(s))
            self.assertIsNotNone(gate, f"{where} has no release-approval-gate.py step")
            self.assertIn(ENVIRONMENT, steps[gate]["run"], where)
            self.assertLess(gate, first_secret, f"{where} reads a signing secret before the approval gate")
            self.assertEqual(steps[gate].get("if"), steps[first_secret].get("if"),
                             f"{where} gate and signing step must run under the same condition")
            self.assertEqual((job.get("permissions") or {}).get("actions"), "read", where)
            gated.append(where)
        self.assertIn("release.yml:signing", gated)
        self.assertIn("distribution.yml:package", gated)

    def test_conditional_environment_covers_every_signing_leg(self):
        job = workflow("distribution.yml")["jobs"]["package"]
        environment = job["environment"]
        signing = next(s for s in job["steps"] if reads_signing_secret(s))
        self.assertEqual(signing["if"], "runner.os == 'macOS' && inputs.tag != ''")
        # Signing runs on macOS legs of tagged builds; the environment must be
        # chosen for exactly those legs (matrix.os is the only job-level proxy).
        self.assertIn("inputs.tag != ''", environment)
        self.assertIn("startsWith(matrix.os, 'macos')", environment)
        self.assertIn(f"'{ENVIRONMENT}'", environment)

    def test_no_workflow_inherits_secrets(self):
        for name, job_id, job in all_jobs():
            self.assertNotEqual(job.get("secrets"), "inherit", f"{name}:{job_id}")

    def test_publish_waits_on_the_gated_package_job(self):
        jobs = workflow("release.yml")["jobs"]
        publishers = [j for j, job in jobs.items() if "publish-release.sh" in json.dumps(job)]
        self.assertEqual(publishers, ["publish"])
        self.assertIn("packages", needs(jobs["publish"]))
        self.assertEqual(jobs["packages"]["uses"], "./.github/workflows/distribution.yml")
        leg = json.dumps(workflow("distribution.yml")["jobs"]["package"]["strategy"])
        self.assertIn('\\"platform\\":\\"macos-arm64\\",\\"optional\\":false', leg)

    def test_nightly_holds_no_signing_secret_and_only_dispatches_release(self):
        nightly = workflow("nightly.yml")
        text = json.dumps(nightly)
        self.assertNotIn(SIGNING_SECRET, text)
        self.assertNotIn("publish-release.sh", text)
        self.assertIn("gh workflow run release.yml", text)


class GateScript(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        bin_dir = Path(self.tmp.name)
        gh = bin_dir / "gh"
        gh.write_text("#!/usr/bin/env python3\nimport os,sys\n"
                      "open(os.environ['GH_ARGS'],'w').write(' '.join(sys.argv[1:]))\n"
                      "sys.stdout.write(os.environ['GH_BODY'])\nsys.exit(int(os.environ['GH_EXIT']))\n")
        gh.chmod(0o755)
        self.args = bin_dir / "args"
        self.env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", GITHUB_REPOSITORY="owner/repo",
                        GH_ARGS=str(self.args), GH_EXIT="0")

    def gate(self, body, exit_code=0, **env):
        env = dict(self.env, GH_BODY=json.dumps(body) if not isinstance(body, str) else body,
                   GH_EXIT=str(exit_code), **env)
        return subprocess.run(["python3", str(GATE), ENVIRONMENT], env=env, capture_output=True, text=True)

    def protected(self, **overrides):
        body = {"name": ENVIRONMENT, "can_admins_bypass": False, "protection_rules": [
            {"type": "required_reviewers", "prevent_self_review": False,
             "reviewers": [{"type": "User", "reviewer": {"login": "owner"}}]}]}
        body.update(overrides)
        return body

    def test_reviewer_without_bypass_passes(self):
        result = self.gate(self.protected())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS environment=release-signing reviewers=owner", result.stdout)
        self.assertEqual(self.args.read_text(), "api repos/owner/repo/environments/release-signing")

    def test_refusals(self):
        cases = {
            "auto-created environment": ({"name": ENVIRONMENT, "can_admins_bypass": True, "protection_rules": []}, 0),
            "no reviewer": (self.protected(protection_rules=[{"type": "wait_timer", "wait_timer": 5}]), 0),
            "empty reviewer list": (self.protected(protection_rules=[{"type": "required_reviewers", "reviewers": []}]), 0),
            "admin bypass": (self.protected(can_admins_bypass=True), 0),
            "bypass field absent": ({k: v for k, v in self.protected().items() if k != "can_admins_bypass"}, 0),
            "api failure": ("{}", 1),
            "invalid json": ("not json", 0),
        }
        for label, (body, code) in cases.items():
            with self.subTest(label):
                result = self.gate(body, code)
                self.assertNotEqual(result.returncode, 0, label)
                self.assertIn("REFUSED environment=release-signing", result.stderr)

    def test_missing_repository_refuses_before_calling_gh(self):
        env = {k: v for k, v in self.env.items() if k != "GITHUB_REPOSITORY"}
        result = subprocess.run(["python3", str(GATE), ENVIRONMENT], env=dict(env, GH_BODY="{}"),
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.args.exists())


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Every signed publish waits for a human approval (stint 1294).

Lint: every job that can read the release signing identity declares the
`release-signing` environment and runs `release-approval-gate.py` before it
touches a secret; publication depends on such a job; nightly holds no signing
secret; no workflow reads secrets wholesale; only release.yml publishes. Gate:
the script refuses unless the environment names a reviewer, forbids
administrator bypass and restricts deployment refs.
"""
import json
import os
import re
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
# The only jobs that may use the signing environment, with their exact
# environment value. A changed expression must be reviewed here.
GATED_JOBS = {
    "distribution.yml:package":
        "${{ inputs.tag != '' && startsWith(matrix.os, 'macos') && 'release-signing' || '' }}",
    "release.yml:signing": "release-signing",
}
# Jobs that may write repository contents (tags, releases).
CONTENT_WRITERS = {"nightly.yml:tag", "release.yml:publish"}
RELEASE_PUBLISH = re.compile(r"publish-release\.sh|gh release (create|upload|edit)|action-gh-release|/releases\b")
WHOLE_SECRETS = re.compile(r"tojson\s*\(\s*secrets\s*\)|secrets\s*\[", re.I)
RUN_ANYWAY = re.compile(r"always\s*\(|failure\s*\(|cancelled\s*\(", re.I)


def workflow(name):
    return yaml.safe_load((WORKFLOWS / name).read_text())


def workflow_paths():
    return sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))


def all_jobs():
    for path in workflow_paths():
        for job_id, job in (yaml.safe_load(path.read_text()).get("jobs") or {}).items():
            yield path.name, job_id, job


def reads_signing_secret(value):
    return SIGNING_SECRET in json.dumps(value)


def needs(job):
    value = job.get("needs") or []
    return [value] if isinstance(value, str) else value


class WorkflowLint(unittest.TestCase):
    def test_signing_environment_and_secrets_are_confined_to_gated_jobs(self):
        seen = set()
        for name, job_id, job in all_jobs():
            where = f"{name}:{job_id}"
            uses_environment = ENVIRONMENT in json.dumps(job.get("environment", ""))
            if not (uses_environment or reads_signing_secret(job)):
                continue
            if "uses" in job and not uses_environment:
                # Caller jobs cannot declare an environment; the called job must.
                self.assertEqual(job["uses"], "./.github/workflows/distribution.yml", where)
                continue
            self.assertIn(where, GATED_JOBS, f"{where} uses the signing environment or secrets outside the gated jobs")
            self.assertEqual(job.get("environment"), GATED_JOBS[where], f"{where} environment changed")
            steps = job.get("steps") or []
            gate = next((i for i, s in enumerate(steps) if "release-approval-gate.py" in str(s.get("run", ""))), None)
            self.assertIsNotNone(gate, f"{where} has no release-approval-gate.py step")
            self.assertIn(f"release-approval-gate.py {ENVIRONMENT}", steps[gate]["run"], where)
            self.assertNotIn("continue-on-error", steps[gate], where)
            for i, step in enumerate(steps):
                if reads_signing_secret(step):
                    self.assertLess(gate, i, f"{where} reads a signing secret before the approval gate")
                    self.assertEqual(steps[gate].get("if"), step.get("if"),
                                     f"{where} gate and signing step must run under the same condition")
            self.assertEqual((job.get("permissions") or {}).get("actions"), "read", where)
            seen.add(where)
        self.assertEqual(seen, set(GATED_JOBS))

    def test_signing_condition_matches_the_environment_expression(self):
        job = workflow("distribution.yml")["jobs"]["package"]
        signing = [s for s in job["steps"] if reads_signing_secret(s)]
        self.assertTrue(signing)
        for step in signing:
            self.assertEqual(step["if"], "runner.os == 'macOS' && inputs.tag != ''")

    def test_no_workflow_reads_secrets_wholesale(self):
        for path in workflow_paths():
            self.assertIsNone(WHOLE_SECRETS.search(path.read_text()), f"{path.name} reads secrets by index or toJSON")
        for name, job_id, job in all_jobs():
            self.assertNotEqual(job.get("secrets"), "inherit", f"{name}:{job_id}")

    def test_only_release_publish_publishes_and_it_waits_on_the_gated_package_job(self):
        publishers = [f"{n}:{j}" for n, j, job in all_jobs() if RELEASE_PUBLISH.search(json.dumps(job))]
        self.assertEqual(publishers, ["release.yml:publish"])
        writers = {f"{n}:{j}" for n, j, job in all_jobs()
                   if (job.get("permissions") or {}) == "write-all"
                   or (isinstance(job.get("permissions"), dict) and job["permissions"].get("contents") == "write")}
        self.assertLessEqual(writers, CONTENT_WRITERS)
        for path in workflow_paths():
            top = yaml.safe_load(path.read_text()).get("permissions")
            self.assertFalse(top == "write-all" or (isinstance(top, dict) and top.get("contents") == "write"), path.name)
        jobs = workflow("release.yml")["jobs"]
        publish, packages = jobs["publish"], jobs["packages"]
        self.assertIn("packages", needs(publish))
        self.assertIsNone(RUN_ANYWAY.search(str(publish.get("if", ""))), "publish must not run after a failed package job")
        self.assertNotIn("continue-on-error", packages)
        self.assertEqual(packages["uses"], "./.github/workflows/distribution.yml")
        leg = json.dumps(workflow("distribution.yml")["jobs"]["package"]["strategy"])
        self.assertIn('\\"platform\\":\\"macos-arm64\\",\\"optional\\":false', leg)

    def test_nightly_holds_no_signing_secret_and_only_dispatches_release(self):
        text = json.dumps(workflow("nightly.yml"))
        self.assertNotIn(SIGNING_SECRET, text)
        self.assertNotIn(ENVIRONMENT, text)
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
        body = {"name": ENVIRONMENT, "can_admins_bypass": False,
                "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True},
                "protection_rules": [
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
            "no branch policy": (self.protected(deployment_branch_policy=None), 0),
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

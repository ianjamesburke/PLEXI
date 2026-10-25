"""Offline proof for babysitter routing, graph and exact-head gates."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("babysitter-policy.py")
spec = importlib.util.spec_from_file_location("babysitter_policy", SCRIPT)
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.settings = policy.config()
        self.head = "a" * 40
        self.base = "b" * 40
        self.record = {
            "size": "S", "head": self.head, "base_head": self.base, "stints": ["0986"],
            "worker_alias": "cos", "tester_alias": "agy", "exhausted": [],
            "local": {"head": self.head, "build": True, "test": True, "clippy": True},
            "checks_head": self.head,
            "validation": {"head": self.head, "verdict": "PASS", "install_head": self.head,
                           "worker_vendor": "codex", "tester_vendor": "gemini"},
            "waivers": [],
        }
        self.pr = {"baseRefName": "feature/alpha-cleanup", "headRefOid": self.head}
        self.checks = [{"name": "required", "bucket": "pass"}]

    def test_sizes_route_distinct_models_and_independent_vendors(self):
        for size, expected in (("S", "cos"), ("M", "com"), ("L", "col")):
            result = policy.route(self.settings, size)
            self.assertEqual(result["worker"], expected)
            self.assertNotEqual(result["worker_vendor"], result["tester_vendor"])

    def test_fallback_and_exhaustion(self):
        fallback = policy.route(self.settings, "S", ["cos"])
        self.assertEqual(fallback["worker"], "cs")
        self.assertNotEqual(fallback["worker_vendor"], fallback["tester_vendor"])
        with self.assertRaises(policy.PolicyError):
            policy.route(self.settings, "S", ["cos", "cs", "agym"])
        with self.assertRaises(policy.PolicyError):
            policy.route(self.settings, "L", ["agy", "cs", "agym", "cos", "agyl", "cl"])
        with self.assertRaises(policy.PolicyError):
            policy.route(self.settings, "M", worker_alias="unknown")

    def test_identical_merge_gates_for_all_sizes(self):
        for size in "SML":
            record = json.loads(json.dumps(self.record))
            record["size"] = size
            record["worker_alias"] = {"S": "cos", "M": "com", "L": "col"}[size]
            self.assertEqual(policy.check_gate(self.settings, record, self.pr, self.checks,
                                               self.base, "batch"), self.head)
            for key in ("build", "test", "clippy"):
                bad = json.loads(json.dumps(record))
                bad["local"][key] = False
                with self.assertRaises(policy.PolicyError):
                    policy.check_gate(self.settings, bad, self.pr, self.checks, self.base, "batch")

    def test_changed_head_base_ci_install_and_self_validation_reject(self):
        for change in (
            lambda record, pr: pr.update(headRefOid="c" * 40),
            lambda record, pr: record.update(base_head="c" * 40),
            lambda record, pr: record.update(checks_head="c" * 40),
            lambda record, pr: record["validation"].update(install_head="c" * 40),
            lambda record, pr: record["validation"].update(tester_vendor="codex"),
            lambda record, pr: record["validation"].update(verdict="FAIL"),
        ):
            record = json.loads(json.dumps(self.record))
            pr = dict(self.pr)
            change(record, pr)
            with self.assertRaises(policy.PolicyError):
                policy.check_gate(self.settings, record, pr, self.checks, self.base, "batch")
        with self.assertRaises(policy.PolicyError):
            policy.check_gate(self.settings, self.record, self.pr,
                              [{"bucket": "pending"}], self.base, "batch")

    def test_waivers_require_pre_dispatch_human_ruling(self):
        record = json.loads(json.dumps(self.record))
        record["dispatched_at"] = "2026-10-25T01:00:00+00:00"
        record["waivers"] = [{"kind": "pre_existing_alpha", "evidence": "run/alpha.log",
                              "ruling": "accept flake", "approved_by": "Ian",
                              "ruling_at": "2026-10-25T00:00:00+00:00",
                              "alpha_head": "d" * 40, "check_name": "required"}]
        failed = [{"name": "required", "bucket": "fail"}]
        policy.check_gate(self.settings, record, self.pr, failed, self.base, "batch")
        for key in ("evidence", "ruling", "approved_by", "ruling_at", "alpha_head", "check_name"):
            bad = json.loads(json.dumps(record))
            bad["waivers"][0].pop(key)
            with self.assertRaises(policy.PolicyError):
                policy.check_gate(self.settings, bad, self.pr, failed, self.base, "batch")
        record["waivers"][0]["ruling_at"] = "2026-10-25T02:00:00+00:00"
        with self.assertRaises(policy.PolicyError):
            policy.check_gate(self.settings, record, self.pr, failed, self.base, "batch")

    def test_final_aggregate_gate_requires_final_install_and_alpha_base(self):
        pr = {"baseRefName": "alpha", "headRefOid": self.head}
        record = dict(self.record, aggregate_head=self.head, integrated_batches=[100])
        policy.check_gate(self.settings, record, pr, self.checks, self.base, "final")
        pr["baseRefName"] = "feature/alpha-cleanup"
        with self.assertRaises(policy.PolicyError):
            policy.check_gate(self.settings, record, pr, self.checks, self.base, "final")
        pr["baseRefName"] = "alpha"
        with self.assertRaises(policy.PolicyError):
            policy.check_gate(self.settings, self.record, pr, self.checks, self.base, "final")

    def test_live_gate_rechecks_head_after_ci(self):
        first = {**self.pr, "state": "OPEN"}
        second = {**first, "headRefOid": "c" * 40}
        responses = [json.dumps(first), f"{self.base}\trefs/heads/feature/alpha-cleanup",
                     json.dumps(second)]
        with patch.object(policy, "command", side_effect=responses), patch.object(
            policy, "read_checks", return_value=self.checks
        ):
            with self.assertRaisesRegex(policy.PolicyError, "changed during final gate"):
                policy.live_gate(self.settings, self.record, 100, "batch")

    def test_aggregate_integration_records_result_without_alpha_mutation(self):
        first = {**self.pr, "state": "OPEN"}
        commands = []

        def fake_command(*args):
            commands.append(args)
            if args[:3] == ("git", "worktree", "list"):
                return "worktree /tmp/aggregate\nbranch refs/heads/feature/alpha-cleanup"
            if args[:4] == ("git", "-C", "/tmp/aggregate", "status"):
                return ""
            if args[:3] == ("git", "ls-remote", "origin"):
                return f"{self.base}\trefs/heads/feature/alpha-cleanup"
            if args[:4] == ("git", "-C", "/tmp/aggregate", "rev-parse"):
                if args[-1] == "HEAD^":
                    return self.base
                return self.base if not any(command[:4] == ("gh", "pr", "merge", "100") for command in commands) else "d" * 40
            if args[:4] == ("gh", "pr", "view", "100"):
                if "state,mergeCommit" in args:
                    return json.dumps({"state": "MERGED", "mergeCommit": {"oid": "d" * 40}})
                return json.dumps(first)
            if args[:4] == ("gh", "pr", "checks", "100"):
                return json.dumps(self.checks)
            return ""

        with tempfile.TemporaryDirectory() as directory, patch.object(
            policy, "command", side_effect=fake_command
        ), patch.object(policy, "read_checks", return_value=self.checks), patch.object(
            policy, "alpha_root", return_value=Path("/tmp/alpha")
        ), patch.object(policy, "preflight", return_value={"size": "S"}):
            manifest = Path(directory) / "aggregate.json"
            self.assertEqual(policy.integrate(self.settings, self.record, 100, manifest), "d" * 40)
            saved = json.loads(manifest.read_text())
            self.assertEqual(saved["batches"][0]["source_head"], self.head)
            self.assertEqual(saved["batches"][0]["aggregate_head"], "d" * 40)
        self.assertIn(("gh", "pr", "merge", "100", "--squash", "--match-head-commit", self.head), commands)
        self.assertFalse(any("alpha" == arg for command in commands for arg in command))

    def test_preflight_blocks_products_until_0991_done(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tasks = root / "tasks"
            tasks.mkdir()
            sprint = root / "s03.md"
            sprint.write_text("- [0991](../tasks/0991-gate.md)\n- [0986](../tasks/0986-zoom.md)\n")
            gate = tasks / "0991-gate.md"
            gate.write_text("---\nstatus: todo\nsize: s\nblocked_by: []\n---\n## Done-When\n")
            product = tasks / "0986-zoom.md"
            product.write_text("---\nstatus: todo\nsize: s\nblocked_by:\n  - 991\n---\n## Done-When\n")
            with self.assertRaises(policy.PolicyError):
                policy.preflight(self.settings, tasks, sprint, ["0986"])
            gate.write_text(gate.read_text().replace("status: todo", "status: done"))
            self.assertEqual(policy.preflight(self.settings, tasks, sprint, ["0986"])["tasks"],
                             ["0986"])
            product.write_text(product.read_text().replace("  - 991\n", ""))
            with self.assertRaises(policy.PolicyError):
                policy.preflight(self.settings, tasks, sprint, ["0986"])
            product.write_text(product.read_text().replace("blocked_by:\n", "blocked_by:\n  - 991\n  - 986\n"))
            with self.assertRaisesRegex(policy.PolicyError, "cycle"):
                policy.preflight(self.settings, tasks, sprint, ["0986"])


if __name__ == "__main__":
    unittest.main()

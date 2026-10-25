"""Offline proof for babysitter routing, graph and exact-head gates."""

import importlib.util
import json
import os
import subprocess
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
        pair_fallback = policy.route(self.settings, "M", ["agy", "cs", "agym"])
        self.assertEqual((pair_fallback["worker"], pair_fallback["tester"]), ("cm", "cos"))
        large_fallback = policy.route(self.settings, "L", ["col", "agy"])
        self.assertEqual((large_fallback["worker"], large_fallback["tester"]), ("agyl", "cos"))
        self.assertEqual(large_fallback["worker_vendor"], "claude")
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

    def test_waivers_cannot_be_self_attested_to_automated_merge(self):
        record = json.loads(json.dumps(self.record))
        record["waivers"] = [{"kind": "pre_existing_alpha", "evidence": "run/alpha.log",
                              "ruling": "accept flake", "approved_by": "Ian",
                              "ruling_at": "2026-10-25T00:00:00+00:00",
                              "alpha_head": "d" * 40, "check_name": "required"}]
        failed = [{"name": "required", "bucket": "fail"}]
        with self.assertRaises(policy.PolicyError):
            policy.check_gate(self.settings, record, self.pr, failed, self.base, "batch")
        with self.assertRaises(policy.PolicyError):
            policy.check_gate(self.settings, record, self.pr, self.checks, self.base, "batch")
        record["waivers"] = []
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
        parent = [self.base]
        already_merged = [False]

        def fake_command(*args):
            commands.append(args)
            if args[:3] == ("git", "worktree", "list"):
                return "worktree /tmp/aggregate\nbranch refs/heads/feature/alpha-cleanup"
            if args[:4] == ("git", "-C", "/tmp/aggregate", "status"):
                return ""
            if args[:3] == ("git", "ls-remote", "origin"):
                return f"{self.base}\trefs/heads/feature/alpha-cleanup"
            if args[:4] == ("git", "-C", "/tmp/aggregate", "rev-parse"):
                if args[-1].endswith("^"):
                    return parent[0]
                return ("d" * 40 if already_merged[0] or any(
                    command[:4] == ("gh", "pr", "merge", "100") for command in commands
                ) else self.base)
            if args[:4] == ("gh", "pr", "view", "100"):
                if "state,mergeCommit" in args:
                    return json.dumps({"state": "MERGED", "mergeCommit": {"oid": "d" * 40}})
                if already_merged[0] and "state,baseRefName,headRefOid,mergeCommit" in args:
                    return json.dumps({**self.pr, "state": "MERGED",
                                       "mergeCommit": {"oid": "d" * 40}})
                return json.dumps(first)
            if args[:4] == ("gh", "pr", "checks", "100"):
                return json.dumps(self.checks)
            return ""

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sprint = root / ".stint/sprints/s03.md"
            sprint.parent.mkdir(parents=True)
            sprint.write_text("- [0991](../tasks/0991-gate.md)\n- [0986](../tasks/0986-zoom.md)\n")
            manifest = root / "aggregate.json"
            with patch.object(policy, "command", side_effect=fake_command), patch.object(
                policy, "read_checks", return_value=self.checks
            ), patch.object(policy, "alpha_root", return_value=root), patch.object(
                policy, "preflight", return_value={"size": "S"}
            ):
                self.assertEqual(policy.integrate(self.settings, self.record, 100, manifest), "d" * 40)
                self.assertIn(("gh", "pr", "merge", "100", "--squash", "--match-head-commit", self.head), commands)
                commands.clear()
                parent[0] = "e" * 40
                raced = root / "race.json"
                with self.assertRaisesRegex(policy.PolicyError, "recorded for recovery"):
                    policy.integrate(self.settings, self.record, 100, raced)
                self.assertTrue(json.loads(raced.read_text())["batches"][0]["base_race"])
                with self.assertRaises(policy.PolicyError):
                    policy.verify_manifest(json.loads(raced.read_text()),
                                           {"integrated_batches": [100]}, "d" * 40)
                commands.clear()
                parent[0] = self.base
                already_merged[0] = True
                recovered = root / "recovered.json"
                self.assertEqual(policy.integrate(self.settings, self.record, 100, recovered), "d" * 40)
                self.assertFalse(any(command[:3] == ("gh", "pr", "merge") for command in commands))
            saved = json.loads(manifest.read_text())
            self.assertEqual(saved["batches"][0]["source_head"], self.head)
            self.assertEqual(saved["batches"][0]["aggregate_head"], "d" * 40)
            self.assertEqual(saved["requested_stints"], ["0986"])
        self.assertFalse(any("alpha" == arg for command in commands for arg in command))

    def test_final_manifest_binds_every_batch_and_requested_task(self):
        manifest = {"topology": "aggregate", "requested_stints": ["0986", "0987"],
                    "batches": [{"pr": 100, "base_before": "b" * 40,
                                 "aggregate_head": "c" * 40, "stints": ["0986"]},
                                {"pr": 101, "base_before": "c" * 40,
                                 "aggregate_head": self.head, "stints": ["0987"]}],
                    "parked": []}
        record = {"integrated_batches": [100, 101]}
        self.assertTrue(policy.verify_manifest(manifest, record, self.head))
        for bad in (
            {**record, "integrated_batches": [100]},
            {**record, "integrated_batches": [100, 101, 102]},
        ):
            with self.assertRaises(policy.PolicyError):
                policy.verify_manifest(manifest, bad, self.head)
        with self.assertRaises(policy.PolicyError):
            policy.verify_manifest(manifest, record, "d" * 40)
        manifest["requested_stints"].append("0988")
        with self.assertRaises(policy.PolicyError):
            policy.verify_manifest(manifest, record, self.head)

    def test_base_race_recovery_requires_final_live_gate_and_preserves_evidence(self):
        manifest = {"topology": "aggregate", "requested_stints": ["0986"],
                    "batches": [{"pr": 100, "base_before": self.base,
                                 "actual_parent": "c" * 40, "base_race": True,
                                 "aggregate_head": self.head, "stints": ["0986"]}],
                    "parked": []}
        record = {**self.record, "aggregate_head": self.head, "integrated_batches": [100]}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "aggregate.json"
            path.write_text(json.dumps(manifest))
            with self.assertRaises(policy.PolicyError):
                policy.verify_manifest(manifest, record, self.head)
            with patch.object(policy, "live_gate", side_effect=policy.PolicyError("stale install")):
                with self.assertRaises(policy.PolicyError):
                    policy.recover_base_race(self.settings, record, 200, path)
            self.assertNotIn("recovery_head", json.loads(path.read_text())["batches"][0])
            with patch.object(policy, "live_gate", return_value=self.head) as live:
                self.assertEqual(policy.recover_base_race(self.settings, record, 200, path), self.head)
                self.assertEqual(live.call_args.args[3], "final")
                self.assertTrue(live.call_args.kwargs["allow_unresolved"])
            saved = json.loads(path.read_text())
            self.assertEqual(saved["batches"][0]["actual_parent"], "c" * 40)
            self.assertEqual(saved["batches"][0]["recovery_head"], self.head)
            self.assertTrue(saved["batches"][0]["base_race"])
            self.assertTrue(policy.verify_manifest(saved, record, self.head))

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
            product.write_text(product.read_text().replace("status: todo", "status: in-progress"))
            self.assertEqual(policy.preflight(self.settings, tasks, sprint, ["0986"])["tasks"],
                             ["0986"])
            product.write_text(product.read_text().replace("blocked_by:\n", "blocked_by:\n  - 986\n"))
            with self.assertRaisesRegex(policy.PolicyError, "cycle"):
                policy.preflight(self.settings, tasks, sprint, ["0986"])

    def test_preflight_accepts_pruned_0991_edge_only_once_0991_done(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tasks = root / "tasks"
            tasks.mkdir()
            sprint = root / "s03.md"
            sprint.write_text("- [0991](../tasks/0991-gate.md)\n- [0986](../tasks/0986-zoom.md)\n")
            gate = tasks / "0991-gate.md"
            gate.write_text("---\nstatus: todo\nsize: s\nblocked_by: []\n---\n## Done-When\n")
            product = tasks / "0986-zoom.md"
            product.write_text("---\nstatus: todo\nsize: s\nblocked_by: []\n---\n## Done-When\n")
            with self.assertRaisesRegex(policy.PolicyError, "0991 dependency"):
                policy.preflight(self.settings, tasks, sprint, ["0986"])
            gate.write_text(gate.read_text().replace("status: todo", "status: done"))
            self.assertEqual(policy.preflight(self.settings, tasks, sprint, ["0986"])["tasks"],
                             ["0986"])
            product.write_text(product.read_text().replace("## Done-When\n", ""))
            with self.assertRaisesRegex(policy.PolicyError, "Done-When"):
                policy.preflight(self.settings, tasks, sprint, ["0986"])

    def test_lane_start_stops_before_worktree_when_preflight_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "wtp-called"
            git = root / "git"
            git.write_text("#!/bin/sh\nif [ \"$3\" = branch ]; then printf '%s\\n' \"${FAKE_BRANCH:-feature/alpha-cleanup}\"; else printf 'worktree /tmp/alpha\\nbranch refs/heads/alpha\\nworktree /tmp/aggregate\\nbranch refs/heads/feature/alpha-cleanup\\n'; fi\n")
            uv = root / "uv"
            uv.write_text("#!/bin/sh\nexit 7\n")
            wtp = root / "wtp"
            wtp.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
            for executable in (git, uv, wtp):
                executable.chmod(0o755)
            result = subprocess.run(("bash", str(SCRIPT.with_name("bs-start.sh")), "--base",
                                     "feature/alpha-cleanup", "0986"), capture_output=True,
                                    text=True, env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}"})
            self.assertEqual(result.returncode, 7)
            self.assertFalse(marker.exists())
            wrong_branch = subprocess.run(("bash", str(SCRIPT.with_name("bs-start.sh")), "--base",
                                           "feature/alpha-cleanup", "0986"), capture_output=True,
                                          text=True, env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                                                          "FAKE_BRANCH": "alpha"})
            self.assertNotEqual(wrong_branch.returncode, 0)
            self.assertIn("must run the helper", wrong_branch.stderr)
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()

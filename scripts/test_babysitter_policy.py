"""Offline proof for babysitter routing, graph and exact-head gates."""

import copy
import importlib.util
import io
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


# Synthetic routing is fixed so operational preference changes cannot alter gate evidence.
TEST_SETTINGS = {
    "authorization": {"auto_merge": True},
    "roles": {
        "head": ["agy", "col", "cm", "agym"],
        "worker_small": ["agyf", "cos", "cs", "pi"],
        "worker_medium": ["agyf", "com", "cm", "pi"],
        "worker_large": ["agyg", "col", "agyl", "cl", "pi"],
        "tester": ["agy", "cs", "agym", "cos"],
    },
    "engines": {
        "alias_vendor": {"cos": "codex", "com": "codex", "col": "codex",
                         "cs": "claude", "cm": "claude", "cl": "claude",
                         "agy": "gemini", "agym": "claude", "agyl": "claude",
                         "agyf": "gemini", "agyg": "gemini", "pi": "deepseek"},
        "alias_model": {"cos": "Luna", "com": "Terra", "col": "Astra",
                        "cs": "Sonnet", "cm": "Opus", "cl": "Opus",
                        "agy": "Gemini", "agym": "Sonnet", "agyl": "Opus",
                        "agyf": "Gemini Flash", "agyg": "Gemini High",
                        "pi": "DeepSeek V4.1 Flash"},
        "alias_engine": {"c": "claude", "co": "codex", "agy": "agy", "pi": "pi"},
        "launch": {"claude": "{alias} {prompt}", "codex": "{alias} {prompt}",
                   "agy": "{alias} -i {prompt}",
                   "pi": "{alias} --model openrouter/deepseek/deepseek-v4.1-flash {prompt}"},
        "working_footer": {"claude": "working", "codex": "working",
                           "agy": "working", "pi": "working"},
    },
    "topology": {"mode": "aggregate", "branch": "feature/alpha-cleanup",
                 "final_base": "alpha", "require_final_install": True},
}


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.settings = copy.deepcopy(TEST_SETTINGS)
        self.head = "a" * 40
        self.base = "b" * 40
        self.record = {
            "size": "S", "head": self.head, "base_head": self.base, "stints": ["0986"],
            "worker_alias": "agyf", "tester_alias": "cs", "exhausted": [],
            "local": {"head": self.head, "build": True, "test": True, "clippy": True},
            "checks_head": self.head,
            "validation": {"head": self.head, "verdict": "PASS", "install_head": self.head,
                           "worker_vendor": "gemini", "tester_vendor": "claude"},
            "waivers": [],
        }
        self.pr = {"baseRefName": "feature/alpha-cleanup", "headRefOid": self.head}
        self.checks = [{"name": "required", "bucket": "pass"}]

    def test_live_config_has_configured_aliases_and_independent_routes(self):
        settings = policy.config()
        engines = settings["engines"]
        for role, aliases in settings["roles"].items():
            self.assertTrue(aliases, role)
            for alias in aliases:
                with self.subTest(role=role, alias=alias):
                    for mapping in ("alias_vendor", "alias_model"):
                        self.assertTrue(engines[mapping].get(alias), (mapping, alias))
                    prefixes = [p for p in engines["alias_engine"] if alias.startswith(p)]
                    self.assertTrue(prefixes, alias)
                    engine = engines["alias_engine"][max(prefixes, key=len)]
                    self.assertTrue(engines["launch"].get(engine), engine)
                    self.assertTrue(engines["working_footer"].get(engine), engine)
        for size in "SML":
            with self.subTest(size=size):
                selected = policy.route(settings, size)
                self.assertNotEqual(selected["worker_vendor"], selected["tester_vendor"])

    def test_sizes_route_distinct_models_and_independent_vendors(self):
        for size, expected in (("S", "agyf"), ("M", "agyf"), ("L", "agyg")):
            result = policy.route(self.settings, size)
            self.assertEqual(result["worker"], expected)
            self.assertNotEqual(result["worker_vendor"], result["tester_vendor"])

    def test_fallback_and_exhaustion(self):
        fallback = policy.route(self.settings, "S", ["agyf"])
        self.assertEqual((fallback["worker"], fallback["tester"]), ("cos", "agy"))
        self.assertNotEqual(fallback["worker_vendor"], fallback["tester_vendor"])
        pair_fallback = policy.route(self.settings, "M", ["cs"])
        self.assertEqual((pair_fallback["worker"], pair_fallback["tester"]), ("agyf", "agym"))
        self.assertEqual(pair_fallback["worker_vendor"], "gemini")
        medium_fallback = policy.route(self.settings, "M", ["agyf", "com"])
        self.assertEqual((medium_fallback["worker"], medium_fallback["tester"]), ("cm", "agy"))
        large_fallback = policy.route(self.settings, "L", ["agyg", "col"])
        self.assertEqual((large_fallback["worker"], large_fallback["tester"]), ("agyl", "agy"))
        self.assertEqual(large_fallback["worker_vendor"], "claude")
        for size, spent in (("S", ["agyf", "cos", "cs"]), ("M", ["agyf", "com", "cm"]),
                            ("L", ["agyg", "col", "agyl", "cl"])):
            last = policy.route(self.settings, size, spent)
            self.assertEqual((last["worker"], last["worker_vendor"]), ("pi", "deepseek"))
            self.assertNotEqual(last["tester_vendor"], "deepseek")
            with self.assertRaises(policy.PolicyError):
                policy.route(self.settings, size, spent + ["pi"])
        with self.assertRaises(policy.PolicyError):
            policy.route(self.settings, "M", worker_alias="unknown")

    def test_pi_is_last_worker_entry_and_fully_configured(self):
        engines = self.settings["engines"]
        for role in ("worker_small", "worker_medium", "worker_large"):
            self.assertEqual(self.settings["roles"][role][-1], "pi")
        for role in ("tester", "head"):
            self.assertNotIn("pi", self.settings["roles"][role])
        self.assertEqual((engines["alias_vendor"]["pi"], engines["alias_model"]["pi"]),
                         ("deepseek", "DeepSeek V4.1 Flash"))
        prefixes = engines["alias_engine"]
        for role in self.settings["roles"].values():
            for alias in role:
                engine = prefixes[max((p for p in prefixes if alias.startswith(p)), key=len)]
                self.assertIn(engine, engines["launch"])
                self.assertIn(engine, engines["working_footer"])
        self.assertEqual(prefixes[max((p for p in prefixes if "pi".startswith(p)), key=len)], "pi")
        self.assertIn("openrouter/deepseek/deepseek-v4.1-flash", engines["launch"]["pi"])

    def test_pi_worker_cannot_be_validated_by_deepseek(self):
        record = json.loads(json.dumps(self.record))
        record.update(worker_alias="pi", tester_alias="agy")
        record["validation"].update(worker_vendor="deepseek", tester_vendor="gemini")
        self.assertEqual(policy.check_gate(self.settings, record, self.pr, self.checks,
                                           self.base, "batch"), self.head)
        record["validation"]["tester_vendor"] = "deepseek"
        with self.assertRaisesRegex(policy.PolicyError, "self-validation"):
            policy.check_gate(self.settings, record, self.pr, self.checks, self.base, "batch")
        self.assertEqual(policy.route(self.settings, "S", ["agyf", "cos", "cs"])["tester"], "agy")

    def test_gemini_worker_is_never_validated_by_gemini(self):
        for size in "SML":
            for exhausted in ([], ["cs"], ["cs", "agym"], ["cs", "agym", "cos"]):
                try:
                    result = policy.route(self.settings, size, exhausted)
                except policy.PolicyError:
                    continue
                self.assertNotEqual(result["worker_vendor"], result["tester_vendor"])
        with self.assertRaises(policy.PolicyError):
            policy.route(self.settings, "S", ["cs", "agym", "cos"], worker_alias="agyf")

    def test_identical_merge_gates_for_all_sizes(self):
        for size in "SML":
            record = json.loads(json.dumps(self.record))
            record["size"] = size
            record["worker_alias"] = {"S": "agyf", "M": "agyf", "L": "agyg"}[size]
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
            lambda record, pr: record["validation"].update(tester_vendor="gemini"),
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
        cleanup_error = [None]
        active_manifest = [None]
        cleanup_names = ("reap-merged-worktrees.sh", "clean-stale-targets.sh", "channel-clean-merged.sh")

        def fake_command(*args):
            commands.append(args)
            if args[0] == "bash":
                self.assertTrue(active_manifest[0].exists())
                self.assertEqual(json.loads(active_manifest[0].read_text())["batches"][0]["aggregate_head"], "d" * 40)
                if Path(args[1]).name == cleanup_names[0] and cleanup_error[0]:
                    raise cleanup_error[0]
                return ""
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
            active_manifest[0] = manifest
            cleanup_commands = [("bash", str(root / "scripts" / name)) for name in cleanup_names]
            with patch.object(policy, "command", side_effect=fake_command), patch.object(
                policy, "read_checks", return_value=self.checks
            ), patch.object(policy, "alpha_root", return_value=root), patch.object(
                policy, "preflight", return_value={"size": "S"}
            ):
                self.assertEqual(policy.integrate(self.settings, self.record, 100, manifest), "d" * 40)
                self.assertIn(("gh", "pr", "merge", "100", "--squash", "--match-head-commit", self.head), commands)
                self.assertEqual([cmd for cmd in commands if cmd[0] == "bash"], cleanup_commands)
                for error in (subprocess.CalledProcessError(1, cleanup_commands[0]), OSError("cleanup unavailable")):
                    with self.subTest(cleanup_error=type(error).__name__):
                        commands.clear()
                        cleanup_error[0] = error
                        active_manifest[0] = root / f"cleanup-{type(error).__name__}.json"
                        with patch("sys.stderr", new_callable=io.StringIO) as stderr:
                            self.assertEqual(policy.integrate(self.settings, self.record, 100, active_manifest[0]), "d" * 40)
                        self.assertEqual([cmd for cmd in commands if cmd[0] == "bash"], cleanup_commands)
                        self.assertEqual(stderr.getvalue().count("warn: cleanup reap-merged-worktrees.sh failed:"), 1)
                        self.assertEqual(len(stderr.getvalue().splitlines()), 1)
                cleanup_error[0] = None
                commands.clear()
                parent[0] = "e" * 40
                raced = root / "race.json"
                active_manifest[0] = raced
                with self.assertRaisesRegex(policy.PolicyError, "recorded for recovery"):
                    policy.integrate(self.settings, self.record, 100, raced)
                self.assertEqual([cmd for cmd in commands if cmd[0] == "bash"], cleanup_commands)
                self.assertTrue(json.loads(raced.read_text())["batches"][0]["base_race"])
                with self.assertRaises(policy.PolicyError):
                    policy.verify_manifest(json.loads(raced.read_text()),
                                           {"integrated_batches": [100]}, "d" * 40)
                commands.clear()
                parent[0] = self.base
                already_merged[0] = True
                recovered = root / "recovered.json"
                active_manifest[0] = recovered
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

    def test_ledger_summary_matches_real_trial_ledger(self):
        fixture = SCRIPT.with_name("testdata") / "babysitter-ledger"
        summary = policy.ledger_summary(fixture)
        self.assertEqual(list(summary), ["agyf"])
        self.assertEqual(summary["agyf"], {
            "attempts": 3, "batches": 2,
            "first_round_pass": {"count": 1, "rate": 0.5},
            "avg_rounds_to_green": 1.5, "parks": 0, "crossovers": 0,
            "usage_limit_hits": 0, "wall_minutes": 162.0, "follow_up_stints": 0})

    def test_ledger_summary_counts_follow_up_stints_per_alias(self):
        fixture = SCRIPT.with_name("testdata") / "babysitter-ledger" / "LEDGER.jsonl"
        real = [json.loads(row) for row in fixture.read_text().splitlines() if row.strip()]
        synthetic = dict(real[0], batch="9999", stints=["9999"], round=1,
                         follow_up_stints=["1100", "1101"])
        omitted_empty = dict(real[1], follow_up_stints=[])
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "LEDGER.jsonl").write_text(
                "\n".join(json.dumps(row) for row in real + [synthetic, omitted_empty]) + "\n")
            summary = policy.ledger_summary(Path(directory))
        self.assertEqual(summary["agyf"]["follow_up_stints"], 2)
        self.assertEqual(summary["agyf"]["attempts"], 5)

    def test_ledger_summary_counts_crossovers_parks_and_limits(self):
        def line(batch, alias, round_, verdict, outcome, **extra):
            return json.dumps({"batch": batch, "stints": [batch], "worker_alias": alias,
                               "round": round_, "verdict": verdict, "outcome": outcome,
                               "usage_limit_hit": extra.get("limit", False),
                               "wall_minutes": 10.0})
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "LEDGER.jsonl").write_text("\n".join([
                line("1", "agyf", 1, "FAIL", "crossed_over"),
                line("1", "cos", 2, "PASS", "merged"),
                line("2", "agyf", 1, "FAIL", "follow_up", limit=True),
                line("2", "agyf", 2, "FAIL", "parked")]) + "\n")
            summary = policy.ledger_summary(Path(directory))
        self.assertEqual(summary["agyf"]["crossovers"], 1)
        self.assertEqual(summary["agyf"]["parks"], 1)
        self.assertEqual(summary["agyf"]["usage_limit_hits"], 1)
        self.assertEqual(summary["agyf"]["first_round_pass"], {"count": 0, "rate": 0.0})
        self.assertIsNone(summary["agyf"]["avg_rounds_to_green"])
        self.assertEqual(summary["cos"]["avg_rounds_to_green"], 2.0)
        with self.assertRaises(policy.PolicyError):
            policy.ledger_summary(Path(directory) / "missing")

    def test_cheap_tier_fail_crosses_over_to_a_strong_non_gemini_tested_alias(self):
        for size in "SM":
            second = policy.route(self.settings, size, ["agyf"])
            self.assertNotIn(second["worker"], ("agyf", "agyg"))
            self.assertNotEqual(second["worker_vendor"], "gemini")
            self.assertNotEqual(second["tester_vendor"], second["worker_vendor"])
        large = policy.route(self.settings, "L", ["agyg"])
        self.assertEqual(large["worker"], "col")
        deep = policy.route(self.settings, "S", ["agyf", "cos", "cs"])
        self.assertEqual(deep["worker"], "pi")

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

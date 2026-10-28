"""Exercise installer scope selection without compiling or installing a host."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/pr-install-pretests.sh"


class InstallPretests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        config = self.root / ".agents/skills/babysitter-v3/RUN_CONFIG.toml"
        config.parent.mkdir(parents=True)
        config.write_text((ROOT / ".agents/skills/babysitter-v3/RUN_CONFIG.toml").read_text())
        (self.root / "scripts/scoped-tests.sh").write_text(
            'printf "%s\\n" "${PR_PRETEST_FIXTURE_FILTERS:-}"\n')
        (self.root / "scripts/cargo-with-lease.sh").write_text(
            'printf "%s\\n" "$*" >> "$PR_PRETEST_FIXTURE_LOG"\n'
            'if [[ "$*" == *--list* && "${PR_PRETEST_FIXTURE_ZERO:-}" != 1 ]]; then\n'
            '  echo "cli::tests::affected: test"\nfi\n')
        self.bin = self.root / "bin"
        self.bin.mkdir()
        just = self.bin / "just"
        just.write_text('#!/bin/sh\nprintf "just %s\\n" "$*" >> "$PR_PRETEST_FIXTURE_LOG"\n')
        just.chmod(0o755)
        self.log = self.root / "commands.log"

    def run_scope(self, scope="scoped", filters="cli::tests::affected", final=False, **extra):
        env = {**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}",
               "PR_PRETEST_FIXTURE_FILTERS": filters, "PR_PRETEST_FIXTURE_LOG": str(self.log), **extra}
        env.pop("PLEXI_PR_TEST_FILTERS", None)
        if "PLEXI_PR_TEST_FILTERS" in extra:
            env["PLEXI_PR_TEST_FILTERS"] = extra["PLEXI_PR_TEST_FILTERS"]
        result = subprocess.run(["bash", str(HELPER), str(self.root), "b" * 40, scope,
                                 "feature/alpha-cleanup" if final else "feature/1097-policy",
                                 "alpha" if final else "feature/alpha-cleanup"],
                                env=env, capture_output=True, text=True)
        commands = self.log.read_text().splitlines() if self.log.exists() else []
        return result, commands

    def test_batch_runs_only_selected_tests_and_sdk_smoke(self):
        result, commands = self.run_scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("matched=1", result.stdout)
        cargo = [line for line in commands if "cargo test" in line]
        self.assertEqual(len(cargo), 2)  # list matches, then execute the selection
        self.assertTrue(all("-- cli::tests::affected" in line for line in cargo))
        self.assertIn("just fetch-python-runtime sdk-smoke", commands)

    def test_scripts_only_never_fall_back_to_full_host_suite(self):
        result, commands = self.run_scope(filters="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any("cargo test" in line for line in commands))
        self.assertIn("just fetch-python-runtime sdk-smoke", commands)

    def test_broad_fallback_requires_reviewed_filters(self):
        result, commands = self.run_scope(filters="::")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(commands, [])
        result, commands = self.run_scope(filters="::", PLEXI_PR_TEST_FILTERS="cli::tests::affected")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all("-- ::" not in line for line in commands))

    def test_zero_matches_and_empty_override_reject(self):
        result, commands = self.run_scope(PR_PRETEST_FIXTURE_ZERO="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("zero tests", result.stderr)
        self.assertTrue(all("--list" in line for line in commands))
        result, _ = self.run_scope(PLEXI_PR_TEST_FILTERS="")
        self.assertNotEqual(result.returncode, 0)

    def test_each_selected_filter_must_match(self):
        result, commands = self.run_scope(filters="cli::tests::affected\ncli::tests::missing")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("filter matched zero tests", result.stderr)
        self.assertTrue(all("--list" in line for line in commands))

    def test_full_is_explicit_and_only_for_final_combined_candidate(self):
        result, commands = self.run_scope(scope="full")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(commands, [])
        result, commands = self.run_scope(scope="full", final=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(line.endswith("cargo test --bin plexi --") for line in commands))

    def test_unknown_scope_rejects_before_any_commands(self):
        result, commands = self.run_scope(scope="skip")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(commands, [])

    def test_recipe_resolves_scope_from_pr_metadata_and_records_it(self):
        recipe = (ROOT / "justfile").read_text().split('pr-install number test_scope="scoped":', 1)[1]
        recipe = recipe.split("\n# Generic channel install.", 1)[0]
        self.assertIn("headRefName,headRefOid,baseRefName,baseRefOid", recipe)
        self.assertIn('"$_wt" "$_base_oid" "{{test_scope}}" "$_head_ref" "$_base_ref"', recipe)
        self.assertIn("test_scope={{test_scope}} test_base=$_base_oid", recipe)


if __name__ == "__main__":
    unittest.main()

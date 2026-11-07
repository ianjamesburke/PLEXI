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
        self.helper = self.root / "scripts/pr-install-pretests.sh"
        self.helper.write_text(HELPER.read_text())
        config = self.root / ".agents/skills/babysitter-v3/RUN_CONFIG.toml"
        config.parent.mkdir(parents=True)
        config.write_text((ROOT / ".agents/skills/babysitter-v3/RUN_CONFIG.toml").read_text())
        (self.root / "scripts/scoped-tests.sh").write_text(
            'printf "%s\\n" "${PR_PRETEST_FIXTURE_FILTERS:-}"\n')
        (self.root / "scripts/cargo-with-lease.sh").write_text(
            'printf "%s\\n" "$*" >> "$PR_PRETEST_FIXTURE_LOG"\n'
            'if [[ "$*" == *--list* && "${PR_PRETEST_FIXTURE_ZERO:-}" != 1 ]]; then\n'
            '  printf "%s\\n" "${PR_PRETEST_FIXTURE_LIST:-cli::tests::affected: test}"\nfi\n')
        (self.root / "scripts/cargo-channel-inputs.py").write_text("# managed helper available\n")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        just = self.bin / "just"
        just.write_text('#!/bin/sh\nprintf "just %s\\n" "$*" >> "$PR_PRETEST_FIXTURE_LOG"\n'
                        'if [ "$1" = scene ]; then exit "${PR_PRETEST_FIXTURE_SCENE_EXIT:-0}"; fi\n')
        just.chmod(0o755)
        git = self.bin / "git"
        git.write_text('#!/bin/sh\nexit "${PR_PRETEST_FIXTURE_GIT_EXIT:-0}"\n')
        git.chmod(0o755)
        self.log = self.root / "commands.log"

    def run_scope(self, scope="scoped", filters="cli::tests::affected", final=False, **extra):
        env = {**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}",
               "PR_PRETEST_FIXTURE_FILTERS": filters, "PR_PRETEST_FIXTURE_LOG": str(self.log),
               "PLEXI_PR_REQUIRED_PTY_FILTERS": "none", "PLEXI_PR_REQUIRED_SCENES": "none", **extra}
        env.pop("PLEXI_PR_TEST_FILTERS", None)
        if "PLEXI_PR_TEST_FILTERS" in extra:
            env["PLEXI_PR_TEST_FILTERS"] = extra["PLEXI_PR_TEST_FILTERS"]
        result = subprocess.run(["bash", str(self.helper), str(self.root), "b" * 40, scope,
                                 "feature/alpha-cleanup" if final else "feature/1097-policy",
                                 "alpha" if final else "feature/alpha-cleanup"],
                                env=env, capture_output=True, text=True)
        commands = self.log.read_text().splitlines() if self.log.exists() else []
        return result, commands

    def test_required_pty_and_scenes_are_executed_and_fail_closed(self):
        env = {"PLEXI_PR_REQUIRED_PTY_FILTERS": "cli::tests::affected",
               "PLEXI_PR_REQUIRED_SCENES": "tests/scenes/example.toml"}
        result, commands = self.run_scope(**env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any("--ignored --list" in line for line in commands))
        self.assertTrue(any("--ignored" in line and "--list" not in line for line in commands))
        self.assertIn("just scene tests/scenes/example.toml", commands)
        self.log.unlink()
        result, commands = self.run_scope(**env, PR_PRETEST_FIXTURE_ZERO="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any("just scene" in line for line in commands))
        self.log.unlink()
        result, _ = self.run_scope(**env, PR_PRETEST_FIXTURE_SCENE_EXIT="1")
        self.assertNotEqual(result.returncode, 0)
        result, _ = self.run_scope(PLEXI_PR_REQUIRED_SCENES="../untracked.toml")
        self.assertNotEqual(result.returncode, 0)
        result, _ = self.run_scope(PLEXI_PR_REQUIRED_SCENES="")
        self.assertNotEqual(result.returncode, 0)
        result, _ = self.run_scope(PLEXI_PR_REQUIRED_PTY_FILTERS="none\ncli::tests::affected")
        self.assertNotEqual(result.returncode, 0)
        result, _ = self.run_scope(**env, PR_PRETEST_FIXTURE_GIT_EXIT="1")
        self.assertNotEqual(result.returncode, 0)

    def test_native_diff_skips_sdk_but_explicit_and_full_require_it(self):
        (self.root / "scripts/pr-sdk-selection.py").write_text('print("skip: native fixture")\n')
        result, commands = self.run_scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("just sdk-smoke", commands)
        self.log.unlink()
        result, commands = self.run_scope(PLEXI_PR_REQUIRE_SDK="1")
        self.assertIn("just sdk-smoke", commands)
        self.log.unlink()
        result, commands = self.run_scope(scope="full", final=True)
        self.assertIn("just sdk-smoke", commands)

    def test_sdk_boundary_selection_is_conservative(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("sdk_selection", ROOT / "scripts/pr-sdk-selection.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for path in ("src/overlays/toolbar.rs",):
            self.assertFalse(module.selection([path])[0], path)
        for path in ("sdk/python/plexi_sdk/app.py", "sdk/protocol/pgap.schema.json",
                     "src/protocol/mod.rs", "src/ui/shared.rs", "src/host/context.rs", "src/host/wasm_python.rs", "apps/test/app.py",
                     "src/app/python_env.rs", "scripts/package.py", "Cargo.toml", "unknown"):
            self.assertTrue(module.selection([path])[0], path)
        self.assertTrue(module.selection([])[0])

    def test_batch_runs_only_selected_tests_and_sdk_smoke(self):
        result, commands = self.run_scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("matched=1", result.stdout)
        cargo = [line for line in commands if "test --bin plexi" in line]
        self.assertTrue(all("python3 scripts/cargo-channel-inputs.py test" in line for line in cargo))
        self.assertEqual(len(cargo), 2)  # list matches, then execute the selection
        self.assertTrue(all("-- cli::tests::affected" in line for line in cargo))
        self.assertIn("just sdk-smoke", commands)

    def test_legacy_worktree_without_helper_retains_raw_cargo_fallback(self):
        (self.root / "scripts/cargo-channel-inputs.py").unlink()
        result, commands = self.run_scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        cargo = [line for line in commands if "test --bin plexi" in line]
        self.assertEqual(len(cargo), 2)
        self.assertTrue(all("cargo test --bin plexi" in line for line in cargo))
        self.assertTrue(all("cargo-channel-inputs" not in line for line in cargo))

    def test_1109_paths_select_reviewed_filters_and_install_pretests(self):
        paths = ["src/overlays/notes_picker.rs", "src/testing/harness_tests.rs",
                 "tests/scenes/notes-picker-shortcut-toggle.toml"]
        selected = subprocess.run(["bash", str(ROOT / "scripts/scoped-tests.sh"), *paths],
                                  cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
        self.assertNotIn("::", selected.splitlines())
        self.assertIn("overlays::", selected.splitlines())
        self.assertIn("testing::harness_tests::", selected.splitlines())
        listing = "\n".join(f"{name}fixture: test" for name in selected.splitlines())
        result, commands = self.run_scope(filters=selected, PR_PRETEST_FIXTURE_LIST=listing)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any("test --bin plexi" in line and "--list" not in line for line in commands))
        unknown = subprocess.run(["bash", str(ROOT / "scripts/scoped-tests.sh"),
                                  "src/new_unknown/file.rs"], cwd=ROOT, capture_output=True,
                                 text=True, check=True).stdout.strip()
        self.assertEqual(unknown, "::")
        result, _ = self.run_scope(filters=unknown)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("broad selection requires reviewed", result.stderr)

    def test_scripts_only_never_fall_back_to_full_host_suite(self):
        result, commands = self.run_scope(filters="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any("test --bin plexi" in line for line in commands))
        self.assertIn("just sdk-smoke", commands)

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
        self.assertTrue(any(line.endswith("cargo-channel-inputs.py test --bin plexi --") for line in commands))

    def test_unknown_scope_rejects_before_any_commands(self):
        result, commands = self.run_scope(scope="skip")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(commands, [])

    def test_recipe_resolves_scope_from_pr_metadata_and_records_it(self):
        recipe = (ROOT / "justfile").read_text().split('_pr-prepare number test_scope action:', 1)[1]
        recipe = recipe.split("\n# Generic channel install.", 1)[0]
        self.assertIn("headRefName,headRefOid,baseRefName,baseRefOid", recipe)
        self.assertIn('"$_wt" "$_base_oid" "{{test_scope}}" "$_head_ref" "$_base_ref"', recipe)
        self.assertIn("test_scope={{test_scope}} test_base=$_base_oid", recipe)


if __name__ == "__main__":
    unittest.main()

"""Zero-match filter guard, nearest-name suggestions, test list cache and the path-table parity check."""

import fnmatch
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("filter_check", ROOT / "scripts/filter_check.py")
FC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FC)

NAMES = ["cli::registry::tests::descriptor_names_unique", "cli::registry::tests::lookup_roundtrip",
         "cli::plugin_check::tests::rejects_unknown_manifest_key", "host::model::tests::split_pane",
         "host::model::tests::close_pane", "testing::harness_tests::focus_moves"]
LISTING = "".join(f"{n}: test\n" for n in NAMES) + "6 tests, 0 benchmarks\n"


class Parsing(unittest.TestCase):
    def test_filters_skips_and_flags(self):
        parsed = FC.parse_args(["host::", "--skip", "close", "--test-threads", "1", "--exact", "--nocapture", "x"])
        self.assertEqual(parsed["filters"], ["host::", "x"])
        self.assertEqual(parsed["skips"], ["close"])
        self.assertTrue(parsed["exact"])
        self.assertTrue(FC.parse_args(["--list"])["bypass"])
        self.assertTrue(FC.parse_args(["a", "--ignored"])["ignored"])
        self.assertEqual(FC.parse_args(["--skip=a", "b"])["skips"], ["a"])

    def test_listing_keeps_only_tests(self):
        self.assertEqual(FC.parse_listing(LISTING), NAMES)

    def test_substring_exact_and_skip_semantics(self):
        per, selected = FC.evaluate(NAMES, ["host::model::"], ["close"], False)
        self.assertEqual(per, {"host::model::": 2})
        self.assertEqual(selected, ["host::model::tests::split_pane"])
        per, selected = FC.evaluate(NAMES, ["host::model::tests::split_pane"], [], True)
        self.assertEqual(selected, ["host::model::tests::split_pane"])
        self.assertEqual(FC.evaluate(NAMES, ["split_pane"], [], True)[0], {"split_pane": 0})


class Suggestions(unittest.TestCase):
    def test_invented_names_point_at_real_modules_and_tests(self):
        self.assertIn("cli::registry::", FC.nearest(FC.module_paths(NAMES), "cli_registry", 3))
        self.assertIn("cli::plugin_check::tests::rejects_unknown_manifest_key",
                      FC.nearest(NAMES, "plugin_check_cli", 3))

    def test_typo_resolved_by_edit_distance(self):
        self.assertEqual(FC.nearest(NAMES, "host::modle::", 1)[0].split("::")[0], "host")

    def test_levenshtein(self):
        self.assertEqual(FC.levenshtein("kitten", "sitting"), 3)
        self.assertEqual(FC.levenshtein("", "ab"), 2)


class Cache(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        (self.root / "src").mkdir(parents=True)
        (self.root / "src/main.rs").write_text("fn main() {}\n")
        (self.root / "Cargo.toml").write_text("[package]\n")
        os.environ[FC.CACHE_ENV] = str(Path(self.temp.name) / "cache")
        self.addCleanup(os.environ.pop, FC.CACHE_ENV)

    def test_key_changes_with_source_and_none_without_src(self):
        first = FC.source_key(self.root)
        self.assertEqual(first, FC.source_key(self.root))
        (self.root / "src/main.rs").write_text("fn main() { /* edit */ }\n")
        self.assertNotEqual(first, FC.source_key(self.root))
        self.assertIsNone(FC.source_key(Path(self.temp.name)))

    def test_precheck_unknown_then_validated_then_stale(self):
        parsed = FC.parse_args(["cli::registry::"])
        self.assertEqual(FC.precheck(self.root, parsed, "t"), "unknown")
        self.assertEqual(FC.validate_listing(self.root, parsed, "t", LISTING, store=True), 2)
        self.assertEqual(FC.precheck(self.root, parsed, "t"), "ok")
        self.assertEqual(FC.precheck(self.root, FC.parse_args(["cli_registry"]), "t"), "zero")
        (self.root / "src/main.rs").write_text("fn main() { /* edit */ }\n")
        self.assertEqual(FC.precheck(self.root, FC.parse_args(["cli_registry"]), "t"), "unknown")

    def test_cache_never_trusts_a_mismatched_record(self):
        key = FC.source_key(self.root)
        FC.cache_store(key, NAMES)
        path = FC.cache_dir() / f"{key}.json"
        path.write_text(path.read_text().replace(key, "0" * 64))
        self.assertIsNone(FC.cache_load(key))


class HostFlow(unittest.TestCase):
    """End to end through filter_check.py with fake lease and cargo helpers."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / "src").mkdir()
        (self.root / "src/main.rs").write_text("fn main() {}\n")
        (self.root / "scripts/filter_check.py").write_text((ROOT / "scripts/filter_check.py").read_text())
        (self.root / "scripts/scoped-tests.sh").write_text((ROOT / "scripts/scoped-tests.sh").read_text())
        (self.root / "scripts/scoped-tests.table").write_text((ROOT / "scripts/scoped-tests.table").read_text())
        (self.root / "scripts/docs_gate_scope.py").write_text("import sys\nsys.exit(1)\n")
        (self.root / "scripts/cargo-with-lease.sh").write_text('printf "lease\\n" >> "$FC_LOG"\nexec "$@"\n')
        (self.root / "scripts/cargo-channel-inputs.py").write_text(
            "import os, sys\n"
            "args = sys.argv[1:]\n"
            "open(os.environ['FC_LOG'], 'a').write('cargo ' + ' '.join(args) + '\\n')\n"
            "names = os.environ['FC_NAMES'].split(',')\n"
            "rest = args[args.index('--') + 1:]\n"
            "if '--list' in rest:\n"
            "    print('\\n'.join(n + ': test' for n in names))\n"
            "else:\n"
            "    filters = [a for a in rest if not a.startswith('-')]\n"
            "    ran = [n for n in names if any(f in n for f in filters)] if filters else names\n"
            "    print(f'running {len(ran)} tests')\n"
            "    sys.exit(int(os.environ.get('FC_EXIT', '0')))\n")
        self.log = Path(self.temp.name) / "log"
        self.cache = Path(self.temp.name) / "cache"

    def run_fc(self, *argv, names=None, **extra):
        env = {**os.environ, "FC_LOG": str(self.log), "FC_NAMES": ",".join(names or NAMES),
               FC.CACHE_ENV: str(self.cache), **extra}
        return subprocess.run([sys.executable, str(self.root / "scripts/filter_check.py"), *argv],
                              cwd=self.root, env=env, capture_output=True, text=True)

    def commands(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_zero_match_fails_after_listing_with_suggestions_and_never_runs(self):
        result = self.run_fc("host", "--", "cli_registry")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("filter matched zero tests: cli_registry", result.stderr)
        self.assertIn("cli::registry::", result.stderr)
        self.assertEqual([c for c in self.commands() if c.startswith("cargo") and "--list" not in c], [])

    def test_second_zero_match_fails_before_any_build_from_the_cache(self):
        self.assertEqual(self.run_fc("host", "--", "host::model::").returncode, 0)
        self.log.unlink()
        result = self.run_fc("host", "--", "cli_registry")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("cached test list", result.stderr)
        self.assertEqual(self.commands(), [], "a fresh cache must reject without taking the lease or building")

    def test_fresh_cache_skips_the_listing_and_runs_once(self):
        self.run_fc("host", "--", "host::model::")
        self.log.unlink()
        result = self.run_fc("host", "--", "host::model::")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len([c for c in self.commands() if c.startswith("cargo")]), 1)
        self.assertIn("running 2 tests", result.stdout)

    def test_stale_cache_is_reported_and_relisted(self):
        self.run_fc("host", "--", "host::model::")
        (self.root / "src/main.rs").write_text("fn main() { /* edit */ }\n")
        self.log.unlink()
        result = self.run_fc("host", "--", "host::model::")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("stale", result.stderr)
        self.assertTrue(any("--list" in c for c in self.commands()))

    def test_no_filters_and_list_pass_through_untouched(self):
        result = self.run_fc("host", "--", "--nocapture")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        result = self.run_fc("host", "--", "--list")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_exit_code_of_the_run_is_preserved(self):
        self.assertEqual(self.run_fc("host", "--", "host::model::", FC_EXIT="3").returncode, 3)

    def test_zero_tests_run_with_exit_zero_is_an_error(self):
        # A cache that says the filter matches, while the real binary runs nothing.
        self.run_fc("host", "--", "host::model::")
        result = self.run_fc("host", "--", "host::model::", names=["other::t"])
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("executed zero tests", result.stderr)

    def test_skip_that_excludes_everything_is_an_error(self):
        result = self.run_fc("host", "--", "host::model::", "--skip", "host::")
        self.assertEqual(result.returncode, 1)
        self.assertIn("--skip excluded every one", result.stderr)

    def test_test_for_table_entry_fallback_and_empty(self):
        (self.root / "src/host").mkdir()
        (self.root / "src/host/model.rs").write_text("//\n")
        (self.root / "src/newmod").mkdir()
        (self.root / "src/newmod/thing.rs").write_text("//\n")
        (self.root / "scripts/x.sh").write_text("#\n")
        names = NAMES + ["newmod::thing::tests::works", "ui_tests::focus"]
        result = self.run_fc("for", "src/host/model.rs", names=names)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("scripts/scoped-tests.table filters: host::", result.stderr)
        self.assertNotIn("FALLBACK", result.stderr)
        result = self.run_fc("for", "src/newmod/thing.rs", names=names)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("FALLBACK", result.stderr)
        self.assertIn("newmod::thing::", result.stderr)
        result = self.run_fc("for", "scripts/x.sh")
        self.assertEqual(result.returncode, 0)
        self.assertIn("just test-infra-scripts", result.stderr)
        self.assertEqual(self.run_fc("for", "src/missing.rs").returncode, 2)
        self.assertEqual(self.run_fc("for", "src/main.rs").returncode, 2)
        # a mapped file whose module has no tests is the zero-match error, not a pass
        (self.root / "src/newmod/empty.rs").write_text("//\n")
        self.assertEqual(self.run_fc("for", "src/newmod/empty.rs", names=names).returncode, 1)

    def test_module_filter(self):
        self.assertEqual(FC.module_filter("src/host/model.rs"), "host::model::")
        self.assertEqual(FC.module_filter("src/cli/mod.rs"), "cli::")
        self.assertIsNone(FC.module_filter("src/main.rs"))


class TableParity(unittest.TestCase):
    """The path table is hand-kept; it must not outlive the tree it describes."""

    def rows(self):
        for line in (ROOT / "scripts/scoped-tests.table").read_text().splitlines():
            if line and not line.startswith("#"):
                patterns, _, filters = line.partition("\t")
                yield patterns.split("|"), filters.split()

    def test_every_pattern_matches_a_tracked_file(self):
        tracked = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True,
                                 check=True).stdout.splitlines()
        stale = []
        for patterns, _ in self.rows():
            for pattern in patterns:
                if not any(fnmatch.fnmatchcase(path, pattern) for path in tracked):
                    stale.append(pattern)
        self.assertEqual(stale, [], "scripts/scoped-tests.table patterns match no tracked file: "
                         "update the table in the change that moved or deleted the path")

    def test_a_removed_path_is_detected(self):
        tracked = ["src/host/model.rs"]
        self.assertFalse(any(fnmatch.fnmatchcase(p, "src/removed_module.rs") for p in tracked))

    def test_every_row_has_valid_filters(self):
        for patterns, filters in self.rows():
            for filter_ in filters:
                self.assertFalse(filter_.startswith("-"), filter_)


if __name__ == "__main__":
    unittest.main()

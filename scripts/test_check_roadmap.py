"""Tests for scripts/check_roadmap.py (stint 1437)."""

import contextlib
import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).with_name("check_roadmap.py")
spec = importlib.util.spec_from_file_location("check_roadmap", SCRIPT)
assert spec and spec.loader
cr = importlib.util.module_from_spec(spec)
sys.modules["check_roadmap"] = cr
spec.loader.exec_module(cr)

REAL_ROADMAP = Path(__file__).resolve().parent.parent / "website/src/lib/roadmap.json"


def roadmap(*releases, current="now"):
    return {"currentSlug": current, "releases": list(releases)}


def rel(slug, status, *items):
    return {"slug": slug, "title": slug, "status": status, "purpose": "p", "items": list(items)}


def item(text, status, *stints):
    out = {"text": text, "status": status}
    if stints:
        out["stints"] = list(stints)
    return out


def stints(**states):
    return {sid.lstrip("s"): cr.Stint(sid.lstrip("s"), state, f"title {sid.lstrip('s')}") for sid, state in states.items()}


class CheckRules(unittest.TestCase):
    def run_check(self, data, known, named: frozenset[str] = frozenset()):
        return cr.check(data, known, named, "origin/alpha")

    def test_planned_item_with_every_stint_done_is_drift(self):
        drift, _ = self.run_check(roadmap(rel("now", "next", item("A", "planned", "1001", "1002"))), stints(s1001="done", s1002="done"))
        self.assertEqual(len(drift), 1)
        self.assertIn('item "A" is \'planned\'', drift[0])
        self.assertIn("stint 1001 (title 1001)", drift[0])
        self.assertIn("Set its status to 'in progress'", drift[0])
        self.assertIn("website/src/lib/roadmap.json", drift[0])

    def test_shipped_item_with_open_stint_is_drift_and_names_it(self):
        drift, _ = self.run_check(roadmap(rel("now", "shipped", item("A", "shipped", "1001", "1002"))), stints(s1001="done", s1002="active"))
        self.assertEqual(len(drift), 1)
        self.assertIn("stint 1002 (title 1002) is active", drift[0])

    def test_missing_and_archived_stints_are_drift(self):
        drift, _ = self.run_check(roadmap(rel("now", "next", item("A", "planned", "1001", "9999"))), stints(s1001="archived"))
        self.assertTrue(any("stint 9999, which does not exist" in d for d in drift))
        self.assertTrue(any("stint 1001 (title 1001), which is archived" in d for d in drift))
        self.assertFalse(any("every linked stint is done" in d for d in drift))

    def test_shipped_after_current_release_is_drift(self):
        data = roadmap(rel("now", "shipped"), rel("later", "shipped", item("B", "shipped")))
        drift, _ = self.run_check(data, {})
        self.assertEqual(len(drift), 2)
        self.assertTrue(any("release 'later' is 'shipped'" in d for d in drift))
        self.assertTrue(any('item "B" is \'shipped\' in a release after' in d for d in drift))

    def test_shipped_in_current_or_earlier_release_is_fine(self):
        data = roadmap(rel("old", "shipped", item("A", "shipped", "1001")), rel("now", "shipped", item("B", "shipped")), current="now")
        drift, info = self.run_check(data, stints(s1001="done"))
        self.assertEqual(drift, [])
        self.assertEqual(info, [])

    def test_done_and_named_on_ref_is_eligible_not_drift(self):
        drift, info = self.run_check(roadmap(rel("now", "next", item("A", "in progress", "1001"))), stints(s1001="done"), frozenset({"1001"}))
        self.assertEqual(drift, [])
        self.assertEqual(len(info), 1)
        self.assertIn("eligible", info[0])

    def test_done_but_not_named_on_ref_is_reported(self):
        _, info = self.run_check(roadmap(rel("now", "next", item("A", "in progress", "1001"))), stints(s1001="done"))
        self.assertIn("no commit on origin/alpha names stint 1001", info[0])

    def test_open_planned_item_and_unlinked_item_are_silent(self):
        data = roadmap(rel("now", "next", item("A", "planned", "1001"), item("B", "planned")))
        self.assertEqual(self.run_check(data, stints(s1001="ready")), ([], []))


class Loading(unittest.TestCase):
    def write(self, data):
        f = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
        json.dump(data, f)
        f.close()
        self.addCleanup(os.unlink, f.name)
        return Path(f.name)

    def test_real_roadmap_loads(self):
        data = cr.load_roadmap(REAL_ROADMAP)
        self.assertIn(data["currentSlug"], [r["slug"] for r in data["releases"]])

    def test_unknown_status_and_bad_id_are_errors(self):
        with self.assertRaisesRegex(cr.CheckError, "unknown status 'done'"):
            cr.load_roadmap(self.write(roadmap(rel("now", "next", item("A", "done")))))
        with self.assertRaisesRegex(cr.CheckError, "four-digit"):
            cr.load_roadmap(self.write(roadmap(rel("now", "next", {"text": "A", "status": "planned", "stints": [577]}))))
        with self.assertRaisesRegex(cr.CheckError, "names no release"):
            cr.load_roadmap(self.write(roadmap(rel("now", "next"), current="gone")))

    def test_valid_json_of_the_wrong_shape_is_a_named_error(self):
        bad = {
            "must be an object": [],
            "a 'releases' list": {"currentSlug": "now", "releases": {"now": {}}},
            "every release must be an object": {"currentSlug": "now", "releases": ["now"]},
            "an item that is not an object": roadmap(rel("now", "next", "just text")),
            "optional 'stints' list": roadmap(rel("now", "next", {"text": "A", "status": "planned", "stints": "1001"})),
        }
        for message, data in bad.items():
            with self.subTest(message), self.assertRaisesRegex(cr.CheckError, message):
                cr.load_roadmap(self.write(data))

    def test_stint_list_of_the_wrong_shape_is_a_named_error(self):
        self.assertEqual(cr.parse_stints('[{"id": "1001", "state": "done", "title": "A"}]')["1001"].state, "done")
        for message, text in {
            "printed a dict, expected a list": '{"id": "1001"}',
            "without string 'id', 'state' and 'title'": '[{"state": "done", "title": "A"}]',
            "without string 'id'": '["1001"]',
        }.items():
            with self.subTest(message), self.assertRaisesRegex(cr.CheckError, message):
                cr.parse_stints(text)


class RoadmapSource(unittest.TestCase):
    def test_ref_reads_the_committed_roadmap_not_the_working_tree(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / cr.EDIT_HINT
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps(roadmap(rel("now", "next", item("committed", "planned")))))
            for args in (["init", "-q"], ["add", "-A"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "seed"]):
                subprocess.run(["git", "-C", d, *args], check=True, capture_output=True)
            path.write_text(json.dumps(roadmap(rel("now", "next", item("edited", "planned")))))
            data = cr.load_roadmap_at("HEAD", Path(d))
            self.assertEqual(data["releases"][0]["items"][0]["text"], "committed")
            self.assertEqual(cr.load_roadmap(path)["releases"][0]["items"][0]["text"], "edited")
            with self.assertRaisesRegex(cr.CheckError, "git show no-such-ref:website/src/lib/roadmap.json"):
                cr.load_roadmap_at("no-such-ref", Path(d))


class Cli(unittest.TestCase):
    """Runs the script as `just check-roadmap` does, with a fake `stint` on PATH."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.bin = Path(self.tmp.name) / "bin"
        self.bin.mkdir()
        self.roadmap = Path(self.tmp.name) / "roadmap.json"
        self.roadmap.write_text(json.dumps(roadmap(rel("now", "shipped", item("A", "shipped", "1001")))))

    def fake_stint(self, body):
        exe = self.bin / "stint"
        exe.write_text(f"#!/bin/sh\n{body}\n")
        exe.chmod(exe.stat().st_mode | stat.S_IEXEC)

    def run_cli(self, *args, fixture=True):
        git = subprocess.run(["which", "git"], capture_output=True, text=True, check=True).stdout.strip()
        env = {"PATH": f"{self.bin}:{Path(git).parent}", "HOME": self.tmp.name}
        base = ["--roadmap", str(self.roadmap), "--commits-ref", "HEAD"] if fixture else []
        return subprocess.run([sys.executable, str(SCRIPT), *base, *args], capture_output=True, text=True, env=env)

    def test_first_line_names_the_roadmap_source(self):
        self.fake_stint("echo '[]'")
        both = self.run_cli("--roadmap-ref", "HEAD", "--commits-ref", "HEAD~1", fixture=False).stdout.splitlines()[0]
        self.assertIn("roadmap from committed HEAD:website/src/lib/roadmap.json; commits from HEAD~1;", both)
        working = self.run_cli(fixture=False).stdout.splitlines()[0]
        self.assertIn("roadmap from working tree website/src/lib/roadmap.json; commits from HEAD;", working)

    def test_missing_stint_cli_is_an_error_not_a_pass(self):
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 2, proc.stdout)
        self.assertIn("`stint` CLI is not on PATH", proc.stderr)

    def test_missing_stint_dir_is_an_error(self):
        self.fake_stint('echo "error: no .stint/ directory found" >&2; exit 1')
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 2)
        self.assertIn("no .stint/ directory found", proc.stderr)

    def test_drift_exits_one_and_clean_exits_zero(self):
        self.fake_stint('echo \'[{"id": "1001", "state": "active", "title": "Do A"}]\'')
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertIn("stint 1001 (Do A) is active", proc.stdout)
        self.fake_stint('echo \'[{"id": "1001", "state": "done", "title": "Do A"}]\'')
        self.assertEqual(self.run_cli().returncode, 0)

    def test_wrongly_shaped_stint_output_exits_two_not_drift(self):
        self.fake_stint('echo \'{"tasks": []}\'')
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 2, proc.stdout)
        self.assertIn("expected a list of tasks", proc.stderr)
        self.assertNotIn("DRIFT", proc.stdout)


class CommitNames(unittest.TestCase):
    def test_repo_convention_counts(self):
        cases = {
            "fix(ci): route cargo through inputs (stint 1419) (#3117)": {"1419"},
            "workspace fail-loud (stints 1127, 1259, 1260) (#3088)": {"1127", "1259", "1260"},
            "rows honor grow (stints 1363 and 1358)": {"1363", "1358"},
            "refactor(permissions): admission trace (stint 1238 step 1) (#3063)": {"1238"},
            "Stint: 0560, 0561": {"0560", "0561"},
            "* test(workspace): unusable workspace is named (stint 1127, red)": {"1127"},
        }
        for message, ids in cases.items():
            with self.subTest(message):
                self.assertEqual(cr.stint_ids_in(message), ids)

    def test_years_pr_numbers_and_shas_do_not_count(self):
        for message in (
            "Owner ruling 2026-11-13: stints are linked in 2026",
            "fix #1300 and 1400 (#3088)",
            "revert a1b2c3d41234ef5 and 9bc6beee8",
            "stint pane names now show 1427 rows",
            "stints 2026-11-13 plan",
            "stint 12345",
        ):
            with self.subTest(message):
                self.assertEqual(cr.stint_ids_in(message), set())

    def test_git_log_is_read_from_the_ref(self):
        with tempfile.TemporaryDirectory() as d:
            git = lambda *a: subprocess.run(["git", "-C", d, *a], check=True, capture_output=True)  # noqa: E731
            git("init", "-q")
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m",
                "feat: thing (stints 1127, 1259 and 1260) (#3088)\n\nfix #1300 and 1400\nStint: 0577")
            self.assertEqual(cr.stints_named_on("HEAD", Path(d)), {"1127", "1259", "1260", "0577"})
            with self.assertRaisesRegex(cr.CheckError, "git log no-such-ref"):
                cr.stints_named_on("no-such-ref", Path(d))


class PromoteMain(unittest.TestCase):
    """`promote main` reads the published roadmap on alpha and searches beta's commits."""

    def test_stale_roadmap_on_beta_does_not_block_a_fixed_one_on_alpha(self):
        with tempfile.TemporaryDirectory() as d:
            repo = Path(d)
            git = lambda *a: subprocess.run(["git", "-C", d, "-c", "user.name=t", "-c", "user.email=t@t", *a], check=True, capture_output=True)  # noqa: E731
            path = repo / cr.EDIT_HINT
            path.parent.mkdir(parents=True)
            git("init", "-q", "-b", "alpha")
            path.write_text(json.dumps(roadmap(rel("now", "next", item("A", "planned", "1001")))))
            git("add", "-A")
            git("commit", "-q", "-m", "feat: A (stint 1001)")
            git("branch", "beta")
            path.write_text(json.dumps(roadmap(rel("now", "next", item("A", "in progress", "1001")))))
            git("commit", "-q", "-am", "docs(website): A is in progress")
            known = stints(s1001="done")
            with mock.patch.object(cr, "REPO", repo), mock.patch.object(cr, "load_stints", return_value=known):
                out = io.StringIO()
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(cr.main(["--roadmap-ref", "alpha", "--commits-ref", "beta"]), 0)
                first = out.getvalue().splitlines()[0]
                self.assertIn("roadmap from committed alpha:website/src/lib/roadmap.json; commits from beta;", first)
                self.assertIn("eligible", out.getvalue())
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(cr.main(["--roadmap-ref", "beta", "--commits-ref", "beta"]), 1)


if __name__ == "__main__":
    unittest.main()

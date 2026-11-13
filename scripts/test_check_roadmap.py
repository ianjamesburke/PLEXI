"""Tests for scripts/check_roadmap.py (stint 1437)."""

import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

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
    def run_check(self, data, known, named=frozenset()):
        return cr.check(data, known, set(named), "origin/alpha")

    def test_planned_item_with_every_stint_done_is_drift(self):
        drift, _ = self.run_check(roadmap(rel("now", "next", item("A", "planned", "1001", "1002"))), stints(s1001="done", s1002="done"))
        self.assertEqual(len(drift), 1)
        self.assertIn('item "A" is \'planned\'', drift[0])
        self.assertIn("stint 1001 (title 1001)", drift[0])
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
        drift, info = self.run_check(roadmap(rel("now", "next", item("A", "in progress", "1001"))), stints(s1001="done"), {"1001"})
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

    def run_cli(self, *args):
        git = subprocess.run(["which", "git"], capture_output=True, text=True, check=True).stdout.strip()
        env = {"PATH": f"{self.bin}:{Path(git).parent}", "HOME": self.tmp.name}
        return subprocess.run([sys.executable, str(SCRIPT), "--roadmap", str(self.roadmap), "--ref", "HEAD", *args], capture_output=True, text=True, env=env)

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


class CommitNames(unittest.TestCase):
    def test_stint_ids_are_read_from_stint_lines_only(self):
        with tempfile.TemporaryDirectory() as d:
            def git(*a):
                subprocess.run(["git", "-C", d, *a], check=True, capture_output=True)
            git("init", "-q")
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m",
                "feat: thing (stints 1127, 1259 and 1260) (#3088)\n\nfix #1300 and 1400\nStint: 0577")
            named = cr.stints_named_on("HEAD", Path(d))
        self.assertEqual(named, {"1127", "1259", "1260", "0577"})
        with self.assertRaisesRegex(cr.CheckError, "git log no-such-ref"):
            cr.stints_named_on("no-such-ref", cr.REPO)


if __name__ == "__main__":
    unittest.main()

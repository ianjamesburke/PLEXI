"""run_logged.py: log file, quiet console, bounded failure tail, one final RESULT line, status file."""

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/run_logged.py"


def parse_result(line):
    assert line.startswith("RESULT "), line
    return dict(part.split("=", 1) for part in line.split()[1:])


class RunLogged(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name) / "logs"

    def run_logged(self, name, command, *flags, **kwargs):
        env = {k: v for k, v in os.environ.items() if k != "PLEXI_RUN_LOGGED_ACTIVE"}
        return subprocess.run([sys.executable, str(SCRIPT), "--dir", str(self.dir), *flags, name, "--", *command],
                              capture_output=True, text=True, env=env, **kwargs)

    def last_line(self, result):
        return result.stdout.rstrip("\n").splitlines()[-1]

    def test_pass_quiet_with_result_status_and_full_log(self):
        result = self.run_logged("ok", ["sh", "-c", "for i in 1 2 3; do echo line$i; done; echo warn >&2"])
        self.assertEqual(result.returncode, 0)
        fields = parse_result(self.last_line(result))
        self.assertEqual((fields["name"], fields["status"], fields["exit"]), ("ok", "pass", "0"))
        self.assertTrue(Path(fields["log"]).is_absolute())
        self.assertEqual(sorted(Path(fields["log"]).read_text().split()), ["line1", "line2", "line3", "warn"])
        self.assertNotIn("line1", result.stdout, "quiet mode must not echo command output")
        self.assertEqual((self.dir / "ok.status").read_text().strip(), self.last_line(result))
        self.assertEqual(result.stdout.count("RESULT "), 1)

    def test_failure_keeps_exit_code_prints_only_bounded_error_tail(self):
        script = ("for i in $(seq 1 200); do echo noise $i; done; "
                  "for i in $(seq 1 100); do echo error: broken $i; done; exit 7")
        result = self.run_logged("bad", ["sh", "-c", script])
        self.assertEqual(result.returncode, 7)
        fields = parse_result(self.last_line(result))
        self.assertEqual((fields["status"], fields["exit"]), ("fail", "7"))
        self.assertNotIn("noise", result.stdout)
        self.assertEqual(result.stdout.count("error: broken"), 40)
        self.assertIn("error: broken 100", result.stdout)
        self.assertEqual(len(Path(fields["log"]).read_text().splitlines()), 300)

    def test_failure_without_error_words_shows_last_lines(self):
        result = self.run_logged("quiet-fail", ["sh", "-c", "echo a; echo b; exit 1"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("  b", result.stdout)

    def test_signal_death_maps_to_128_plus_signal(self):
        result = self.run_logged("sig", ["sh", "-c", "kill -9 $$"])
        self.assertEqual(result.returncode, 137)
        self.assertEqual(parse_result(self.last_line(result))["exit"], "137")

    def test_missing_command_reports_fail_127(self):
        result = self.run_logged("nocmd", ["/nonexistent/binary"])
        self.assertEqual(result.returncode, 127)
        self.assertEqual(parse_result(self.last_line(result))["status"], "fail")

    def test_heartbeat_while_running_and_status_says_running(self):
        process = subprocess.Popen([sys.executable, str(SCRIPT), "--dir", str(self.dir), "--heartbeat", "0.5", "slow",
                                   "--", "sh", "-c", "echo started; sleep 2; echo done"],
                                  stdout=subprocess.PIPE, text=True)
        deadline = time.time() + 5
        status = self.dir / "slow.status"
        while time.time() < deadline and not status.exists():
            time.sleep(0.05)
        self.assertIn("status=running", status.read_text())
        self.assertIn("pid=", status.read_text())
        out, _ = process.communicate(timeout=20)
        self.assertIn("run-logged: slow running", out)
        self.assertEqual(parse_result(out.rstrip("\n").splitlines()[-1])["status"], "pass")
        self.assertNotIn("running", status.read_text())

    def test_tee_passes_streams_through_separately_and_still_logs(self):
        result = self.run_logged("tee", ["sh", "-c", "echo out; echo err >&2; exit 3"], "--tee")
        self.assertEqual(result.returncode, 3)
        self.assertEqual(result.stdout.splitlines()[0], "out")
        self.assertEqual(result.stderr.strip(), "err")
        self.assertEqual(parse_result(self.last_line(result))["exit"], "3")
        self.assertEqual(sorted((self.dir / "tee.log").read_text().split()), ["err", "out"])
        self.assertNotIn("error lines from the log", result.stdout, "tee mode adds no tail: the output already passed through")

    def test_nested_wrapper_is_transparent(self):
        inner = [sys.executable, str(SCRIPT), "--dir", str(self.dir), "inner", "--", "echo", "hi"]
        result = self.run_logged("outer", inner, "--tee")
        self.assertEqual(result.stdout.count("RESULT "), 1)
        self.assertFalse((self.dir / "inner.status").exists())

    def test_usage_errors(self):
        for argv in ([], ["name"], ["name", "--"], ["bad name", "--", "true"]):
            result = subprocess.run([sys.executable, str(SCRIPT), "--dir", str(self.dir), *argv],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, argv)

    def test_sigterm_is_forwarded_and_recorded(self):
        process = subprocess.Popen([sys.executable, str(SCRIPT), "--dir", str(self.dir), "term", "--", "sleep", "30"],
                                   stdout=subprocess.PIPE, text=True)
        while not (self.dir / "term.status").exists():
            time.sleep(0.05)
        time.sleep(0.5)
        process.send_signal(signal.SIGTERM)
        out, _ = process.communicate(timeout=20)
        fields = parse_result(out.rstrip("\n").splitlines()[-1])
        self.assertEqual((fields["status"], fields["exit"]), ("fail", "143"))


class RecipeWiring(unittest.TestCase):
    """The long recipes keep their commands and gain only the RESULT trailer."""

    def test_long_recipes_run_through_the_wrapper_in_tee_mode(self):
        lines = (ROOT / "justfile").read_text().splitlines()
        for name in ("release-host-gate", "clippy", "test-host", "pr-install", "pr-candidate"):
            start = next(i for i, l in enumerate(lines) if l.startswith((name + ":", name + " ")))
            body = []
            for line in lines[start + 1:]:
                if not line.startswith("    "):
                    break
                body.append(line)
            self.assertTrue(any("scripts/run_logged.py --tee " + name in l for l in body), name)
        justfile = "\n".join(lines)
        self.assertIn("run-logged name *command", justfile)


if __name__ == "__main__":
    unittest.main()

"""run_logged.py: log file, quiet console, bounded failure tail, one final RESULT line, status file."""

import os
import pty
import select
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(os.environ.get("RUN_LOGGED_UNDER_TEST") or ROOT / "scripts/run_logged.py")  # env: revert proofs


def parse_result(line):
    assert line.startswith("RESULT "), line
    return dict(part.split("=", 1) for part in line.split()[1:])


class Base(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name) / "logs"

    def run_logged(self, name, command, *flags, **kwargs):
        env = {k: v for k, v in os.environ.items() if k != "PLEXI_RUN_LOGGED_ACTIVE"}
        return subprocess.run([sys.executable, str(SCRIPT), "--dir", str(self.dir), *flags, name, "--", *command],
                              capture_output=True, text=True, env=env, start_new_session=True, **kwargs)

    def files(self, name, suffix):
        return sorted(self.dir.glob(f"{name}-*.{suffix}"))

    def last_line(self, result):
        return result.stdout.rstrip("\n").splitlines()[-1]



class RunLogged(Base):
    def test_pass_quiet_with_result_status_and_full_log(self):
        result = self.run_logged("ok", ["sh", "-c", "for i in 1 2 3; do echo line$i; done; echo warn >&2"])
        self.assertEqual(result.returncode, 0)
        fields = parse_result(self.last_line(result))
        self.assertEqual((fields["name"], fields["status"], fields["exit"]), ("ok", "pass", "0"))
        self.assertTrue(Path(fields["log"]).is_absolute())
        self.assertEqual(sorted(Path(fields["log"]).read_text().split()), ["line1", "line2", "line3", "warn"])
        self.assertNotIn("line1", result.stdout, "quiet mode must not echo command output")
        self.assertEqual(self.files("ok", "status")[0].read_text().strip(), self.last_line(result))
        self.assertEqual(Path(fields["log"]).with_suffix(".status").resolve(), self.files("ok", "status")[0].resolve())
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
        while time.time() < deadline and not self.files("slow", "status"):
            time.sleep(0.05)
        status = self.files("slow", "status")[0]
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
        self.assertEqual(sorted(self.files("tee", "log")[0].read_text().split()), ["err", "out"])
        self.assertNotIn("error lines from the log", result.stdout, "tee mode adds no tail: the output already passed through")

    def test_nested_wrapper_is_transparent(self):
        inner = [sys.executable, str(SCRIPT), "--dir", str(self.dir), "inner", "--", "echo", "hi"]
        result = self.run_logged("outer", inner, "--tee")
        self.assertEqual(result.stdout.count("RESULT "), 1)
        self.assertEqual(self.files("inner", "status"), [])

    def test_usage_errors(self):
        for argv in ([], ["name"], ["name", "--"], ["bad name", "--", "true"]):
            result = subprocess.run([sys.executable, str(SCRIPT), "--dir", str(self.dir), *argv],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, argv)

    def test_sigterm_is_forwarded_and_recorded(self):
        process = subprocess.Popen([sys.executable, str(SCRIPT), "--dir", str(self.dir), "term", "--", "sleep", "30"],
                                   stdout=subprocess.PIPE, text=True)
        while not self.files("term", "status"):
            time.sleep(0.05)
        time.sleep(0.5)
        process.send_signal(signal.SIGTERM)
        out, _ = process.communicate(timeout=20)
        fields = parse_result(out.rstrip("\n").splitlines()[-1])
        self.assertEqual((fields["status"], fields["exit"]), ("fail", "143"))


class Hardening(Base):
    """Review findings: unique names, closed downstream, bounded join, unwritable log dir."""

    def test_repeated_and_concurrent_runs_of_one_name_never_share_files(self):
        first = self.run_logged("same", ["sh", "-c", "echo one"])
        second = self.run_logged("same", ["sh", "-c", "echo two; exit 1"])
        logs = self.files("same", "log")
        self.assertEqual(len(logs), 2)
        self.assertEqual(sorted(l.read_text().strip() for l in logs), ["one", "two"])
        self.assertEqual(sorted(l.read_text().split()[2] for l in self.files("same", "status")),
                         ["status=fail", "status=pass"])
        self.assertNotEqual(parse_result(self.last_line(first))["log"], parse_result(self.last_line(second))["log"])
        procs = [subprocess.Popen([sys.executable, str(SCRIPT), "--dir", str(self.dir), "par", "--", "sh", "-c", "sleep 1"],
                                  stdout=subprocess.PIPE, text=True) for _ in range(3)]
        logs = {parse_result(p.communicate(timeout=30)[0].rstrip("\n").splitlines()[-1])["log"] for p in procs}
        self.assertEqual(len(logs), 3)

    def test_quiet_mode_announces_its_files_first(self):
        result = self.run_logged("announce", ["true"])
        first = result.stdout.splitlines()[0]
        self.assertIn("started; log=", first)
        self.assertIn(parse_result(self.last_line(result))["log"], first)

    def test_closed_downstream_does_not_hang_the_wrapper(self):
        command = "head -c 300000 /dev/zero | tr '\\0' x; echo; echo tail >&2; exit 3"
        process = subprocess.Popen([sys.executable, str(SCRIPT), "--dir", str(self.dir), "--tee", "pipe", "--",
                                   "sh", "-c", command], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        process.stdout.close()  # `... | head` that already exited
        self.addCleanup(process.stderr.close)
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            self.fail("wrapper hung on a closed stdout")
        self.assertEqual(process.returncode, 3)
        log = self.files("pipe", "log")[0]
        self.assertGreater(log.stat().st_size, 300000)
        self.assertIn("exit=3", self.files("pipe", "status")[0].read_text())

    def test_descendant_holding_the_pipes_does_not_block_the_verdict(self):
        started = time.time()
        result = self.run_logged("daemon", ["sh", "-c", "(sleep 20) & echo parent-done"], timeout=60)
        self.assertLess(time.time() - started, 15, "must not wait for the background descendant")
        self.assertEqual(parse_result(self.last_line(result))["status"], "pass")
        self.assertIn("descendant still holds", result.stderr)

    def test_sigterm_reaches_the_whole_process_group(self):
        marker = Path(self.temp.name) / "child.pid"
        script = f"sh -c 'echo $$ > {marker}; exec sleep 60' & wait"
        process = subprocess.Popen([sys.executable, str(SCRIPT), "--dir", str(self.dir), "grp", "--", "sh", "-c", script],
                                   stdout=subprocess.PIPE, text=True)
        deadline = time.time() + 10
        while time.time() < deadline and not (marker.exists() and marker.read_text().strip()):
            time.sleep(0.05)
        grandchild = int(marker.read_text())
        process.send_signal(signal.SIGTERM)
        out, _ = process.communicate(timeout=30)
        self.assertEqual(parse_result(out.rstrip("\n").splitlines()[-1])["exit"], "143")
        time.sleep(0.5)
        with self.assertRaises(ProcessLookupError):
            os.kill(grandchild, 0)

    def test_signal_ignoring_group_is_killed_after_the_grace_period(self):
        process = subprocess.Popen([sys.executable, "-c",
                                    "import sys, importlib.util as u; s=u.spec_from_file_location('r', sys.argv[1]); "
                                    "m=u.module_from_spec(s); s.loader.exec_module(m); m.KILL_AFTER_SECONDS=1.0; "
                                    "sys.exit(m.main(sys.argv[2:]))", str(SCRIPT), "--dir", str(self.dir), "stubborn", "--",
                                    "sh", "-c", "trap '' TERM; sleep 60"], stdout=subprocess.PIPE, text=True)
        while not self.files("stubborn", "status"):
            time.sleep(0.05)
        time.sleep(0.5)
        process.send_signal(signal.SIGTERM)
        out, _ = process.communicate(timeout=30)
        self.assertEqual(parse_result(out.rstrip("\n").splitlines()[-1])["exit"], "137")

    def test_unwritable_log_dir_warns_and_still_runs_the_command(self):
        blocker = Path(self.temp.name) / "file-not-dir"
        blocker.write_text("x")
        for flags in ((), ("--tee",)):
            result = subprocess.run([sys.executable, str(SCRIPT), "--dir", str(blocker / "logs"), *flags, "nolog", "--",
                                     "sh", "-c", "echo ran; exit 4"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 4, flags)
            self.assertIn("ran", result.stdout, "output must still reach the console unlogged")
            self.assertIn("running unlogged", result.stderr)
            fields = parse_result(self.last_line(result))
            self.assertEqual((fields["status"], fields["exit"], fields["log"]), ("fail", "4", "-"))

    def test_missing_command_is_reported_as_such_not_as_a_log_failure(self):
        result = self.run_logged("nocmd2", ["/nonexistent/binary"])
        self.assertEqual(result.returncode, 127)
        self.assertIn("cannot start", result.stderr)


class Terminal(Base):
    """A person at a terminal: the command must not be stopped (SIGTTIN) and Ctrl-C must reach it."""

    def on_pty(self, flags, name, shell, marker_input, timeout=30):
        """Run the wrapper as a session leader on a pty. Once the command creates its marker
        file, type marker_input. Returns (pty output, exit status or None if it hung)."""
        marker = Path(self.temp.name) / f"{name}.ready"
        pid, fd = pty.fork()
        if pid == 0:
            os.execv(sys.executable, [sys.executable, str(SCRIPT), "--dir", str(self.dir), *flags, name, "--",
                                      "sh", "-c", shell.replace("MARKER", str(marker))])
        output, sent, deadline = b"", False, time.time() + timeout
        status = None
        while time.time() < deadline:
            if not sent and marker.exists():
                time.sleep(0.3)
                os.write(fd, marker_input)
                sent = True
            if select.select([fd], [], [], 0.2)[0]:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    break
                output += chunk
            done, state = os.waitpid(pid, os.WNOHANG)
            if done:
                status = os.waitstatus_to_exitcode(state)
                break
        end = time.time() + 5
        while status is None and time.time() < end:  # EOF seen: the wrapper is exiting
            done, state = os.waitpid(pid, os.WNOHANG)
            if done:
                status = os.waitstatus_to_exitcode(state)
            else:
                time.sleep(0.05)
        if status is None:
            os.kill(pid, signal.SIGKILL)
            subprocess.run(["pkill", "-KILL", "-f", f"MARKER-{name}"])
            os.waitpid(pid, 0)
        os.close(fd)
        return output.decode(errors="replace"), status

    def test_command_can_read_the_terminal_without_being_stopped(self):
        for flags in ((), ("--tee",)):
            name = "tty-read" + ("-tee" if flags else "")
            output, status = self.on_pty(flags, name, "touch MARKER; read answer < /dev/tty; echo got-$answer", b"hi\n")
            self.assertEqual(status, 0, f"{flags}: hung or failed (SIGTTIN stop?) {output!r}")
            self.assertIn("status=pass", output)

    def test_ctrl_c_reaches_the_command(self):
        for flags in ((), ("--tee",)):
            name = "tty-int" + ("-tee" if flags else "")
            output, status = self.on_pty(flags, name, f"touch MARKER; exec sleep 60 # MARKER-{name}", b"\x03")
            self.assertEqual(status, 130, f"{flags}: {output!r}")
            self.assertIn("exit=130", output)


class RecipeWiring(unittest.TestCase):
    """The long recipes keep their commands and gain only the RESULT trailer."""

    def test_long_recipes_run_through_the_wrapper_in_tee_mode(self):
        lines = (ROOT / "justfile").read_text().splitlines()
        for name in ("release-host-gate", "clippy", "test-host", "test-for", "pr-install", "pr-candidate"):
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

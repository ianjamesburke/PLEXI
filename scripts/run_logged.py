#!/usr/bin/env python3
"""Run a long command with a full log, a quiet console and one machine-readable verdict.

  python3 scripts/run_logged.py [--tee] [--dir DIR] [--heartbeat SECS] <name> -- <command...>
  just run-logged <name> -- <command...>

Every run gets its own files, `<dir>/<name>-<UTC time>-<pid>.log` and the same stem
with `.status`, so concurrent or repeated runs of one name never overwrite each other.
Quiet mode (default, for agents) prints their paths on its first line.

Quiet mode: the full stdout and stderr go to the log; the console gets a one-line
heartbeat every --heartbeat seconds (default 30); on failure it also gets a bounded
tail of the error lines.
Tee mode (--tee, used by the long justfile recipes): output passes through unchanged
on the original stdout and stderr streams and is also logged, so existing consumers
see the same bytes plus the final line. On an interactive terminal tee mode runs the
command attached to the terminal with no capture (colors and progress bars intact) and
reports `log=-`. A downstream that closes early (`... | head`) stops receiving output;
the command keeps running and keeps logging.
If the log directory is not writable the run warns, runs unlogged (output passes
through, quiet mode included) and reports `log=-`; it never refuses to run the command.

Both modes end stdout with exactly one line and write the same line to the status file:

  RESULT name=<name> status=<pass|fail> exit=<code> seconds=<n> log=<abs path or ->

The status file holds `status=running pid=<pid>` from the start, so a later process can
tell "still running" (pid alive) from "wrapper killed" (pid dead, status still running).
The wrapper exits with the command's exit code (128+signal for a signalled command).
The command runs in its own process group (except attached to a terminal); SIGINT,
SIGTERM and SIGHUP sent to the wrapper go to that whole group once, and a group that
ignores them is killed after 10 s. SIGKILL cannot be caught, so a killed wrapper leaves
`status=running` with a dead pid. A nested run (the command itself uses run_logged) is
transparent: only the outermost wrapper reports.
"""

import argparse
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

ACTIVE_ENV = "PLEXI_RUN_LOGGED_ACTIVE"
DIR_ENV = "PLEXI_RUN_LOG_DIR"
ERROR_MARKERS = ("error", "failed", "panicked", "failures:", "fatal")
TAIL_LINES = 40
LINE_CLIP = 300
JOIN_SECONDS = 3.0      # how long to wait for output after the command exits
KILL_AFTER_SECONDS = 10.0  # forwarded signal ignored this long: SIGKILL the group


def warn(message):
    print(f"run-logged: warning: {message}", file=sys.stderr, flush=True)


def log_dir(explicit):
    if explicit:
        return Path(explicit).resolve()
    if os.environ.get(DIR_ENV):
        return Path(os.environ[DIR_ENV]).resolve()
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    base = Path(top.stdout.strip()) if top.returncode == 0 and top.stdout.strip() else Path.cwd()
    return (base / ".stint/runs/logs").resolve()


def result_line(name, status, code, seconds, log):
    return f"RESULT name={name} status={status} exit={code} seconds={seconds} log={log}"


def write_status(path, text):
    temp = path.with_name(path.name + f".{os.getpid()}.tmp")
    temp.write_text(text + "\n")
    os.replace(temp, path)


def say(text, stream=None):
    """print that survives a closed stdout/stderr: the verdict must never crash the wrapper."""
    try:
        print(text, file=stream or sys.stdout, flush=True)
    except (BrokenPipeError, OSError):
        try:
            os.dup2(os.open(os.devnull, os.O_WRONLY), (stream or sys.stdout).fileno())
        except OSError:
            pass


def error_tail(log_path):
    """At most TAIL_LINES error lines from the log; the last lines when none look like errors."""
    try:
        lines = log_path.read_text(errors="replace").splitlines()
    except OSError as error:
        return [f"(could not read the log {log_path}: {error})"]
    hits = [l for l in lines if any(m in l.lower() for m in ERROR_MARKERS)]
    chosen = hits[-TAIL_LINES:] if hits else lines[-20:]
    return [l[:LINE_CLIP] for l in chosen]


class Run:
    def __init__(self, name, command, directory, tee, heartbeat):
        self.name, self.command, self.tee, self.heartbeat = name, command, tee, heartbeat
        self.directory = directory
        self.log = None      # Path while logging, None when unlogged
        self.status = None   # Path while the status file is writable
        self.logfile = None
        self.lines = 0
        self.last = ""
        self.lock = threading.Lock()
        self.signalled_at = None

    # --- files -----------------------------------------------------------

    def open_files(self):
        """Create a unique log + status pair; on any failure warn and carry on unlogged."""
        stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + f"-{os.getpid()}"
        try:
            self.directory.mkdir(parents=True, exist_ok=True)
            log = self.directory / f"{self.name}-{stamp}.log"
            self.logfile = open(log, "xb")
            self.log = log
        except OSError as error:
            warn(f"cannot write the log under {self.directory} ({error}); running unlogged, log=-")
            return
        self.status = log.with_suffix(".status")
        self.put_status(result_line(self.name, "running", "-", 0, self.log) + f" pid={os.getpid()}")

    def put_status(self, text):
        if self.status is None:
            return
        try:
            write_status(self.status, text)
        except OSError as error:
            warn(f"cannot write the status file {self.status} ({error}); continuing without it")
            self.status = None

    # --- output ------------------------------------------------------------

    def pump(self, stream, sink):
        """Pass chunks through at once (so \\r progress output is not delayed); log whole lines.
        A failing sink is dropped, never fatal: the command must keep draining and logging."""
        pending = b""
        while True:
            chunk = stream.read1(65536)
            if not chunk:
                break
            if sink is not None:
                try:
                    sink.buffer.write(chunk)
                    sink.buffer.flush()
                except (BrokenPipeError, OSError, ValueError):
                    sink = None
            pending += chunk
            *complete, pending = pending.split(b"\n")
            if complete:
                self.record([line + b"\n" for line in complete])
        if pending:
            self.record([pending + b"\n"])

    def record(self, lines):
        with self.lock:
            if self.logfile is not None:
                try:
                    self.logfile.write(b"".join(lines))
                    self.logfile.flush()
                except (OSError, ValueError):
                    self.logfile = None
            self.lines += len(lines)
            for raw in reversed(lines):
                text = raw.decode(errors="replace").strip()
                if text:
                    self.last = text
                    break

    # --- running -------------------------------------------------------------

    def execute(self):
        interactive = self.tee and sys.stdout.isatty() and sys.stderr.isatty()
        started = time.monotonic()
        if not interactive:
            self.open_files()
        else:
            try:
                self.directory.mkdir(parents=True, exist_ok=True)
                stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + f"-{os.getpid()}"
                self.status = self.directory / f"{self.name}-{stamp}.status"
                self.put_status(result_line(self.name, "running", "-", 0, "-") + f" pid={os.getpid()}")
            except OSError as error:
                warn(f"cannot write the status file under {self.directory} ({error})")
        passthrough = self.tee or self.logfile is None
        if not self.tee and self.log is not None:
            say(f"run-logged: {self.name} started; log={self.log} status={self.status}")
        env = {**os.environ, ACTIVE_ENV: self.name}
        try:
            if interactive:
                process = subprocess.Popen(self.command, env=env)
                threads = []
            else:
                process = subprocess.Popen(self.command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                           process_group=0)
                sinks = (sys.stdout, sys.stderr) if passthrough else (None, None)
                threads = [threading.Thread(target=self.pump, args=(process.stdout, sinks[0]), daemon=True),
                           threading.Thread(target=self.pump, args=(process.stderr, sinks[1]), daemon=True)]
                for t in threads:
                    t.start()
        except OSError as error:
            warn(f"cannot start {self.command[0]!r}: {error}")
            return self.finish(127, started, [])

        def forward(signum, _frame):
            if self.signalled_at is None:
                self.signalled_at = time.monotonic()
            self.deliver(process, interactive, signum)
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            # Attached to a terminal Ctrl-C already reaches the command: never deliver SIGINT twice.
            if interactive and signum == signal.SIGINT:
                signal.signal(signum, lambda *_: None)
            else:
                signal.signal(signum, forward)
        next_beat = time.monotonic() + self.heartbeat
        while process.poll() is None:
            time.sleep(0.2)
            now = time.monotonic()
            if self.signalled_at is not None and now - self.signalled_at > KILL_AFTER_SECONDS:
                self.deliver(process, interactive, signal.SIGKILL)
                self.signalled_at = now
            if not passthrough and now >= next_beat:
                with self.lock:
                    say(f"run-logged: {self.name} running {int(now - started)}s, "
                        f"{self.lines} log lines; last: {self.last[:100]}")
                next_beat = now + self.heartbeat
        deadline = time.monotonic() + JOIN_SECONDS
        for t in threads:
            t.join(max(0.0, deadline - time.monotonic()))
        if any(t.is_alive() for t in threads):
            warn("the command exited but a descendant still holds its output pipes; "
                 "the log may be missing its last lines")
        with self.lock:
            if self.logfile is not None:
                self.logfile.close()
                self.logfile = None
        code = process.returncode
        if code < 0:
            code = 128 + -code
        tail = error_tail(self.log) if (code != 0 and not passthrough and self.log is not None) else []
        return self.finish(code, started, tail)

    @staticmethod
    def deliver(process, interactive, signum):
        try:
            if interactive:
                process.send_signal(signum)
            else:
                os.killpg(process.pid, signum)  # the command leads its own group (process_group=0)
        except OSError:
            pass

    def finish(self, code, started, tail):
        seconds = int(time.monotonic() - started)
        line = result_line(self.name, "pass" if code == 0 else "fail", code, seconds, self.log or "-")
        self.put_status(line)
        if tail:
            say(f"run-logged: {self.name} failed (exit {code}); error lines from the log (max {TAIL_LINES}):")
            for entry in tail:
                say(f"  {entry}")
        say(line)
        return code


def main(argv):
    if "--" not in argv:
        print("run-logged: usage: run_logged.py [--tee] [--dir D] [--heartbeat S] <name> -- <command...>", file=sys.stderr)
        return 2
    split = argv.index("--")
    parser = argparse.ArgumentParser(prog="run_logged.py")
    parser.add_argument("--tee", action="store_true")
    parser.add_argument("--dir")
    parser.add_argument("--heartbeat", type=float, default=30.0)
    parser.add_argument("name")
    options = parser.parse_args(argv[:split])
    command = argv[split + 1:]
    if not command:
        print("run-logged: no command after --", file=sys.stderr)
        return 2
    if not options.name.replace("-", "").replace("_", "").replace(".", "").isalnum():
        print(f"run-logged: name must be letters, digits, '-', '_' or '.': {options.name!r}", file=sys.stderr)
        return 2
    if os.environ.get(ACTIVE_ENV):
        os.execvp(command[0], command)
    return Run(options.name, command, log_dir(options.dir), options.tee, options.heartbeat).execute()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

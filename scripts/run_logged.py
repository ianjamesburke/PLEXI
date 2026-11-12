#!/usr/bin/env python3
"""Run a long command with a full log, a quiet console and one machine-readable verdict.

  python3 scripts/run_logged.py [--tee] [--dir DIR] [--heartbeat SECS] <name> -- <command...>
  just run-logged <name> -- <command...>

Quiet mode (default, for agents): the full stdout and stderr go to
`<dir>/<name>.log`; the console gets a one-line heartbeat every --heartbeat
seconds (default 30); on failure it also gets a bounded tail of the error lines.
Tee mode (--tee, used by the long justfile recipes): output passes through
unchanged on the original stdout and stderr streams and is also logged, so
existing consumers see the same bytes plus the final line. On an interactive
terminal tee mode runs the command attached to the terminal with no capture
(colors and progress bars intact) and reports `log=-`.

Both modes end stdout with exactly one line and write the same line to
`<dir>/<name>.status`:

  RESULT name=<name> status=<pass|fail> exit=<code> seconds=<n> log=<abs path>

The status file holds `status=running pid=<pid>` from the start, so a later
process can tell "still running" (pid alive) from "wrapper killed" (pid dead,
status still running) without reading output. The wrapper exits with the
command's exit code (128+signal for a signalled command). SIGTERM and SIGHUP sent
to the wrapper are forwarded to the command; SIGKILL cannot be caught, so a killed
wrapper leaves `status=running` with a dead pid. A nested run (the
command itself uses run_logged) is transparent: only the outermost wrapper
reports.
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
        self.log = directory / f"{name}.log"
        self.status = directory / f"{name}.status"
        self.lines = 0
        self.last = ""
        self.lock = threading.Lock()

    def pump(self, stream, sink, logfile):
        """Pass chunks through at once (so \\r progress output is not delayed); log whole lines."""
        pending = b""
        while True:
            chunk = stream.read1(65536)
            if not chunk:
                break
            if sink is not None:
                sink.buffer.write(chunk)
                sink.buffer.flush()
            pending += chunk
            *complete, pending = pending.split(b"\n")
            if complete:
                self.record(logfile, [line + b"\n" for line in complete])
        if pending:
            self.record(logfile, [pending + b"\n"])

    def record(self, logfile, lines):
        with self.lock:
            logfile.write(b"".join(lines))
            logfile.flush()
            self.lines += len(lines)
            for raw in reversed(lines):
                text = raw.decode(errors="replace").strip()
                if text:
                    self.last = text
                    break

    def execute(self):
        interactive = self.tee and sys.stdout.isatty() and sys.stderr.isatty()
        started = time.monotonic()
        log_shown = "-" if interactive else str(self.log)
        self.directory.mkdir(parents=True, exist_ok=True)
        write_status(self.status, result_line(self.name, "running", "-", 0, log_shown) + f" pid={os.getpid()}")
        env = {**os.environ, ACTIVE_ENV: self.name}
        try:
            if interactive:
                process = subprocess.Popen(self.command, env=env)
                threads, logfile = [], None
            else:
                logfile = open(self.log, "wb")
                process = subprocess.Popen(self.command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                sinks = (sys.stdout, sys.stderr) if self.tee else (None, None)
                threads = [threading.Thread(target=self.pump, args=(process.stdout, sinks[0], logfile), daemon=True),
                           threading.Thread(target=self.pump, args=(process.stderr, sinks[1], logfile), daemon=True)]
                for t in threads:
                    t.start()
        except OSError as error:
            print(f"run-logged: cannot start {self.command[0]!r}: {error}", file=sys.stderr)
            return self.finish(127, started, log_shown, [])

        def forward(signum, _frame):
            try:
                process.send_signal(signum)
            except OSError:
                pass
        # SIGTERM/SIGHUP are forwarded once. Ctrl-C already reaches the command through the shared
        # foreground process group, so SIGINT is only noted, never delivered a second time.
        for signum in (signal.SIGTERM, signal.SIGHUP):
            signal.signal(signum, forward)
        signal.signal(signal.SIGINT, lambda *_: None)
        next_beat = time.monotonic() + self.heartbeat
        while process.poll() is None:
            time.sleep(0.2)
            if not self.tee and time.monotonic() >= next_beat:
                with self.lock:
                    print(f"run-logged: {self.name} running {int(time.monotonic() - started)}s, "
                          f"{self.lines} log lines; last: {self.last[:100]}", flush=True)
                next_beat = time.monotonic() + self.heartbeat
        for t in threads:
            t.join()
        if logfile:
            logfile.close()
        code = process.returncode
        if code < 0:
            code = 128 + -code
        tail = error_tail(self.log) if (code != 0 and not self.tee and not interactive) else []
        return self.finish(code, started, log_shown, tail)

    def finish(self, code, started, log_shown, tail):
        seconds = int(time.monotonic() - started)
        line = result_line(self.name, "pass" if code == 0 else "fail", code, seconds, log_shown)
        write_status(self.status, line)
        if tail:
            print(f"run-logged: {self.name} failed (exit {code}); error lines from the log (max {TAIL_LINES}):")
            for entry in tail:
                print(f"  {entry}")
        print(line, flush=True)
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

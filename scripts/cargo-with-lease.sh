#!/usr/bin/env bash
# Throttle cargo invocations across worktrees with plain kernel file locks, so
# N concurrent builds (e.g. babysitter lanes) don't OOM the machine. This is a
# throttle, not a correctness lock — it used to be host-arbitrated
# (`plexi lock acquire/release`), but the host keyed ownership to the PANE,
# not the process: a client that died (Ctrl-C, SIGKILL, timeout) left the
# pane registered as a permanent holder and wedged every later build. A
# kernel flock self-releases when the holder process dies by any means,
# including SIGKILL — that's the whole point of this rewrite.
#
# Two permits: permit 0 is the lock file itself ($PLEXI_CARGO_LOCK or
# ~/.plexi/cargo-build.lock, so wrapper copies that predate permits still
# exclude full jobs), permit 1 is `<lock>.1`. Permit 0 serves any job. Permit 1 is reserved
# for jobs the call site declares `--class short`; a full job never takes it,
# so a doc generator or focused test never queues behind a full build.
# Class is declared, never guessed from duration; unmarked calls are `full`.
# Full jobs may take either permit (PLEXI_CARGO_FULL_PERMITS=2, the default):
# two simultaneous cold full release builds were measured on this 16 GiB
# machine at 5.7 GB peak summed RSS, no swap growth, 38% memory free at the
# low point (stint 1250, .stint/runs/1250). Set PLEXI_CARGO_FULL_PERMITS=1 to
# confine full jobs to permit 0. Either way at most two cargo jobs run at once,
# and a full job yields to any waiting short job, so one permit frees up for it.
#
# Each holder writes `<lock>.<permit>.holder` (pid, cwd, command, class,
# start time). Waiters print it. A holder killed by SIGKILL leaves the file
# behind, but waiters only read it while the permit's flock is held, so a stale
# record is never reported and the next holder overwrites it.
#
# Contract: `bash scripts/cargo-with-lease.sh [--class short|full] <command...>`
# runs <command> under a permit and exits with the command's exit code. Every
# justfile call site depends on this staying stable.
set -euo pipefail

job_class=full
if [[ "${1:-}" == "--class" ]]; then
  job_class="${2:-}"
  if [[ "$job_class" != "short" && "$job_class" != "full" ]]; then
    echo "cargo-lease: --class must be 'short' or 'full', got '${job_class}'" >&2
    exit 2
  fi
  shift 2
fi
export PLEXI_CARGO_JOB_CLASS="$job_class"

# Nesting guard: a wrapped command (e.g. `just pr-install`) can itself invoke
# this wrapper. If the marker is already set, this invocation's parent holds
# the lock — run directly instead of trying to re-acquire it, which would
# deadlock against our own ancestor.
if [[ -n "${PLEXI_CARGO_LOCK_HELD:-}" ]]; then
  exec "$@"
fi

# The python helper becomes the lock holder: it opens the lock file, takes an
# exclusive flock, then runs the wrapped command as a child for exactly as
# long as the fd (and thus the lock) is held. No bash trap is involved —
# traps never fire on SIGKILL, which is the failure mode being eliminated.
#
# The python source is captured into a variable (not fed to python on stdin
# via `python3 - <<PY`) so the wrapped command still inherits this script's
# real stdin — a heredoc-on-stdin would hand python's already-consumed pipe
# to the child instead, silently EOF-ing anything the wrapped command reads
# (a codesign/keychain prompt, a cargo prompt, any interactive command).
read -r -d '' PLEXI_CARGO_LEASE_PY <<'PY' || true
import fcntl
import glob
import json
import uuid
import os
import subprocess
import sys
import time

LOCK_BASE = os.environ.get("PLEXI_CARGO_LOCK") or os.path.join(
    os.path.expanduser("~"), ".plexi", "cargo-build.lock"
)
TIMEOUT_SECS = float(os.environ.get("PLEXI_CARGO_LEASE_TIMEOUT_SECS", "900"))
REPORT_SECS = float(os.environ.get("PLEXI_CARGO_LEASE_REPORT_SECS", "30"))
POLL_INTERVAL_SECS = 0.2
JOB_CLASS = os.environ["PLEXI_CARGO_JOB_CLASS"]
FULL_PERMITS = int(os.environ.get("PLEXI_CARGO_FULL_PERMITS", "2"))
if FULL_PERMITS not in (1, 2):
    print(f"cargo-lease: PLEXI_CARGO_FULL_PERMITS must be 1 or 2, got {FULL_PERMITS}", file=sys.stderr)
    sys.exit(2)
# Permit order = preference. Short jobs prefer permit 1 so permit 0 stays free
# for a full build; full jobs use permit 1 only when FULL_PERMITS allows it.
CANDIDATES = [1, 0] if JOB_CLASS == "short" else ([0, 1] if FULL_PERMITS == 2 else [0])

argv = sys.argv[1:]
if not argv:
    print("cargo-lease: no command given", file=sys.stderr)
    sys.exit(2)

def permit_path(index):
    return LOCK_BASE if index == 0 else f"{LOCK_BASE}.{index}"

def holder_path(index):
    return permit_path(index) + ".holder"

os.makedirs(os.path.dirname(LOCK_BASE), exist_ok=True)
lock_fds = {i: os.open(permit_path(i), os.O_CREAT | os.O_RDWR, 0o644) for i in (0, 1)}

started = time.monotonic()
run_id = str(uuid.uuid4())
permit = None
def event(phase, **fields):
    record = {"schema": 1, "id": run_id, "event": phase, "time": time.time(),
              "pid": os.getpid(), "cwd": os.getcwd(), "command": argv,
              "channel": os.environ.get("PLEXI_BUILD_TEST_CHANNEL"),
              "target": os.environ.get("CARGO_TARGET_DIR"),
              "profile": "release" if "--release" in argv else "dev/test",
              "class": JOB_CLASS, "permit": permit, **fields}
    print("cargo-lease: " + json.dumps(record, sort_keys=True), file=sys.stderr, flush=True)
    destination = os.environ.get("PLEXI_BUILD_TELEMETRY")
    if destination:
        try:
            with open(destination, "a") as stream:
                stream.write(json.dumps(record, sort_keys=True) + "\n")
        except OSError as error:
            print(f"cargo-lease: telemetry write failed at {destination}: {error}", file=sys.stderr)

def describe_holders():
    """One line per candidate permit: its live holder, or that it is free."""
    lines = []
    for index in CANDIDATES:
        try:
            fcntl.flock(lock_fds[index], fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(lock_fds[index], fcntl.LOCK_UN)
            lines.append(f"permit {index} free (yielding to a waiting short job)")
            continue
        except BlockingIOError:
            pass
        try:
            with open(holder_path(index)) as stream:
                h = json.load(stream)
            lines.append(
                f"permit {index} held by pid {h['pid']} class={h['class']} for "
                f"{int(time.time() - h['started'])}s: {' '.join(h['command'])} (cwd {h['cwd']})")
        except (OSError, ValueError, KeyError):
            lines.append(f"permit {index} held by an unknown holder (no record)")
    return lines

# A short job that cannot get a permit publishes a beacon: a file it holds an
# flock on for as long as it waits. Full jobs yield while any live beacon
# exists, so the next permit to free up goes to the short job. A beacon left by
# a SIGKILLed waiter is detected by its flock being free and removed.
beacon = None

def short_waiting():
    live = False
    for name in glob.glob(f"{LOCK_BASE}.shortwait.*"):
        try:
            fd = os.open(name, os.O_RDWR)
        except OSError:
            continue
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            os.unlink(name)  # nobody holds it: stale
        except BlockingIOError:
            live = True
        except OSError:
            pass
        finally:
            os.close(fd)
    return live

def publish_beacon():
    global beacon
    if beacon is None:
        path = f"{LOCK_BASE}.shortwait.{os.getpid()}"
        fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o644)
        fcntl.flock(fd, fcntl.LOCK_EX)
        beacon = (fd, path)

def drop_beacon():
    global beacon
    if beacon is not None:
        try:
            os.unlink(beacon[1])
        except OSError:
            pass
        os.close(beacon[0])
        beacon = None

def write_holder(index):
    record = {"pid": os.getpid(), "cwd": os.getcwd(), "command": argv,
              "class": JOB_CLASS, "started": time.time(), "permit": index,
              "target": os.environ.get("CARGO_TARGET_DIR")}
    temp = f"{holder_path(index)}.{os.getpid()}.tmp"
    try:
        with open(temp, "w") as stream:
            json.dump(record, stream)
        os.replace(temp, holder_path(index))
    except OSError as error:
        print(f"cargo-lease: holder record write failed at {holder_path(index)}: {error}", file=sys.stderr)

event("request")
deadline = started + TIMEOUT_SECS
next_report = started + REPORT_SECS
while permit is None:
    if JOB_CLASS == "short" or not short_waiting():
        for index in CANDIDATES:
            try:
                fcntl.flock(lock_fds[index], fcntl.LOCK_EX | fcntl.LOCK_NB)
                permit = index
                break
            except BlockingIOError:
                continue
    if permit is not None:
        break
    if JOB_CLASS == "short":
        publish_beacon()
    now = time.monotonic()
    if now >= deadline:
        drop_beacon()
        holders = "; ".join(describe_holders())
        event("timeout", queue_seconds=now - started, holders=holders)
        print(
            f"cargo-lease: timed out after {TIMEOUT_SECS:.0f}s waiting for a "
            f"class={JOB_CLASS} permit ({', '.join(permit_path(i) for i in CANDIDATES)}): {holders}",
            file=sys.stderr,
        )
        sys.exit(1)
    if now >= next_report:
        print(f"cargo-lease: waiting {int(now - started)}s for a class={JOB_CLASS} permit; "
              + "; ".join(describe_holders()), file=sys.stderr, flush=True)
        next_report = now + REPORT_SECS
    time.sleep(POLL_INTERVAL_SECS)

drop_beacon()
acquired_at = time.monotonic()
write_holder(permit)
event("acquire", queue_seconds=acquired_at - started)
env = dict(os.environ)
env["PLEXI_CARGO_LOCK_HELD"] = "1"
try:
    # stdin/stdout/stderr are inherited unchanged (Popen default), so the
    # wrapped command sees this script's real stdin, not python's.
    result = subprocess.run(argv, env=env)
finally:
    try:
        os.unlink(holder_path(permit))
    except OSError:
        pass
rc = result.returncode
if rc < 0:
    # subprocess.run reports death-by-signal as a negative returncode
    # (Python convention); translate to the shell convention (128 + signum)
    # so a SIGKILLed wrapped command reports 137, Ctrl-C reports 130, etc.,
    # matching what every justfile call site and CI expect.
    rc = 128 + (-rc)
event("finish", queue_seconds=acquired_at - started, execution_seconds=time.monotonic() - acquired_at, result=rc)
sys.exit(rc)
PY

exec python3 -c "$PLEXI_CARGO_LEASE_PY" "$@"

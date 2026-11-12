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
# Three permits. Permits 0 and 1 are general: any job may take them, and full
# jobs only ever use these (permit 1 only when PLEXI_CARGO_FULL_PERMITS=2, the
# default). Permit 2 is short-only: only a job the call site declares
# `--class short` can take it, so a doc generator or focused test starts
# within seconds even while two full builds run. Short jobs prefer permit 2 and
# fall back to a free general permit. Class is declared, never guessed from
# duration; unmarked calls are `full`. Permit 0 is the lock file itself
# ($PLEXI_CARGO_LOCK or ~/.plexi/cargo-build.lock), so wrapper copies that
# predate permits still exclude full jobs; permit 1 is `<lock>.1`, permit 2 is
# `<lock>.short`. At most FULL_PERMITS full jobs plus the short-only permit run at once
# (shorts may also borrow a free general permit; never more than three jobs).
# Measured on the 16 GiB dev machine (stint 1250): two cold release builds plus
# a cold test-profile compile peaked at 6.9 GB summed RSS, no swap growth.
# Mark `short` by cost: cold wall time near 2 min or less and no whole-host
# build (doc generators). Anything that builds the host bin or its tests is full.
# PLEXI_CARGO_FULL_PERMITS=1 confines full jobs to permit 0.
# CARGO_BUILD_JOBS defaults to cpu_count // (FULL_PERMITS + 1) so permits also bound rustc fan-out.
#
# Each holder writes `<permit file>.holder` (pid, cwd, command, class,
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
SHORT_ONLY = 2
FULL_PERMITS = int(os.environ.get("PLEXI_CARGO_FULL_PERMITS", "2"))
if FULL_PERMITS not in (1, 2):
    print(f"cargo-lease: PLEXI_CARGO_FULL_PERMITS must be 1 or 2, got {FULL_PERMITS}", file=sys.stderr)
    sys.exit(2)
GENERAL = [0, 1] if FULL_PERMITS == 2 else [0]
# Permit order = preference: short jobs try the short-only permit first so the
# general permits stay free for full builds.
CANDIDATES = [SHORT_ONLY] + GENERAL if JOB_CLASS == "short" else GENERAL

argv = sys.argv[1:]
if not argv:
    print("cargo-lease: no command given", file=sys.stderr)
    sys.exit(2)

def permit_path(index):
    return {0: LOCK_BASE, 1: f"{LOCK_BASE}.1", SHORT_ONLY: f"{LOCK_BASE}.short"}[index]

def holder_path(index):
    return permit_path(index) + ".holder"

os.makedirs(os.path.dirname(LOCK_BASE), exist_ok=True)
lock_fds = {i: os.open(permit_path(i), os.O_CREAT | os.O_RDWR, 0o644) for i in (0, 1, SHORT_ONLY)}

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
            lines.append(f"permit {index} free")
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
    for index in CANDIDATES:
        try:
            fcntl.flock(lock_fds[index], fcntl.LOCK_EX | fcntl.LOCK_NB)
            permit = index
            break
        except BlockingIOError:
            continue
    if permit is not None:
        break
    now = time.monotonic()
    if now >= deadline:
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

acquired_at = time.monotonic()
write_holder(permit)
event("acquire", queue_seconds=acquired_at - started)
env = dict(os.environ)
env["PLEXI_CARGO_LOCK_HELD"] = "1"
# A permit bounds concurrent jobs, not threads: without a cap each permitted
# cargo fans out one rustc per core. Split cores across the permits that can
# run at once (general permits plus the short-only one). An explicit
# CARGO_BUILD_JOBS from the caller wins.
env.setdefault("CARGO_BUILD_JOBS", str(max(1, (os.cpu_count() or 1) // (FULL_PERMITS + 1))))
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

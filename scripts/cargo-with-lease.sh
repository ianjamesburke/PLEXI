#!/usr/bin/env bash
# Serialize cargo invocations across worktrees with a plain kernel file lock,
# so N concurrent builds (e.g. babysitter lanes) don't OOM the machine. This
# is a throttle, not a correctness lock — it used to be host-arbitrated
# (`plexi lock acquire/release`), but the host keyed ownership to the PANE,
# not the process: a client that died (Ctrl-C, SIGKILL, timeout) left the
# pane registered as a permanent holder and wedged every later build. A
# kernel flock self-releases when the holder process dies by any means,
# including SIGKILL — that's the whole point of this rewrite.
#
# Contract: `bash scripts/cargo-with-lease.sh <command...>` runs <command>
# under the lock and exits with the command's exit code. Every justfile call
# site depends on this staying stable.
set -euo pipefail

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
import os
import subprocess
import sys
import time

LOCK_PATH = os.environ.get("PLEXI_CARGO_LOCK") or os.path.join(
    os.path.expanduser("~"), ".plexi", "cargo-build.lock"
)
TIMEOUT_SECS = float(os.environ.get("PLEXI_CARGO_LEASE_TIMEOUT_SECS", "900"))
POLL_INTERVAL_SECS = 0.2

argv = sys.argv[1:]
if not argv:
    print("cargo-lease: no command given", file=sys.stderr)
    sys.exit(2)

os.makedirs(os.path.dirname(LOCK_PATH), exist_ok=True)
lock_fd = os.open(LOCK_PATH, os.O_CREAT | os.O_RDWR, 0o644)

deadline = time.monotonic() + TIMEOUT_SECS
acquired = False
while True:
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        acquired = True
        break
    except BlockingIOError:
        if time.monotonic() >= deadline:
            break
        time.sleep(POLL_INTERVAL_SECS)

if not acquired:
    print(
        f"cargo-lease: timed out after {TIMEOUT_SECS:.0f}s waiting for "
        f"the lock at {LOCK_PATH}",
        file=sys.stderr,
    )
    sys.exit(1)

env = dict(os.environ)
env["PLEXI_CARGO_LOCK_HELD"] = "1"
# stdin/stdout/stderr are inherited unchanged (Popen default), so the
# wrapped command sees this script's real stdin, not python's.
result = subprocess.run(argv, env=env)
rc = result.returncode
if rc < 0:
    # subprocess.run reports death-by-signal as a negative returncode
    # (Python convention); translate to the shell convention (128 + signum)
    # so a SIGKILLed wrapped command reports 137, Ctrl-C reports 130, etc.,
    # matching what every justfile call site and CI expect.
    rc = 128 + (-rc)
sys.exit(rc)
PY

exec python3 -c "$PLEXI_CARGO_LEASE_PY" "$@"

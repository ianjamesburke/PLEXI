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

# CPU throttle: run the build under background QoS, which on Apple Silicon
# confines the process tree to the efficiency cores and leaves the performance
# cores free for whatever the human is doing. Measured on this 10-core M1 Pro,
# eight busy loops draw 380% unclamped and 111% clamped. The clamp is inherited
# by children, so applying it to the lock holder covers cargo, every rustc it
# spawns, and the test binary underneath.
#
# `taskpolicy -c utility` was measured and does NOT throttle (748%) — utility
# QoS still schedules on performance cores. Background is the only clamp that
# actually keeps the machine responsive, so it is the one used here.
#
# Builds are meaningfully slower this way. That is the trade being made: this
# is a background build queue on an interactive machine. Set
# PLEXI_BUILD_FULL_SPEED=1 for a one-off build that should use every core.
CARGO_LEASE_PREFIX=()
if [[ -z "${PLEXI_BUILD_FULL_SPEED:-}" ]] && [[ -x /usr/sbin/taskpolicy ]]; then
  CARGO_LEASE_PREFIX=(/usr/sbin/taskpolicy -b)
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

# `${a[@]+"${a[@]}"}` rather than a bare `"${a[@]}"`: under `set -u`, bash 3.2
# (still /bin/bash on macOS) treats an empty array's [@] expansion as an unbound
# variable and aborts. This form expands to nothing when the array is empty.
exec ${CARGO_LEASE_PREFIX[@]+"${CARGO_LEASE_PREFIX[@]}"} python3 -c "$PLEXI_CARGO_LEASE_PY" "$@"

#!/usr/bin/env bash
# Linux bringup smoke check — see docs/linux-support-plan.md, Phase 4.
#
# Drives a real host through the CLI on an X display and asserts a success
# signal at every step. Exits non-zero at the first failure, naming the step.
#
#   bash scripts/linux-smoke.sh [path-to-plexi-binary]
#
# Defaults to target/release/plexi. Channel-agnostic: every command goes
# through the binary under test, which resolves its own channel and profile.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLEXI="${1:-$REPO_ROOT/target/release/plexi}"
WORK="$(mktemp -d -t plexi-linux-smoke-XXXXXX)"
STEP_N=0
FAILED=0
HOST_STARTED=0

# The caller is very likely itself inside a Plexi pane. Those vars would point
# every command at the *caller's* host instead of the binary under test.
unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_CONTEXT_ROOT PLEXI_CONTEXT_ID \
      PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_PANE_ID

ok()   { STEP_N=$((STEP_N + 1)); printf 'ok %d: %s\n' "$STEP_N" "$1"; }
fail() {
  STEP_N=$((STEP_N + 1)); FAILED=1
  printf 'FAIL %d: %s\n' "$STEP_N" "$1" >&2
  if [[ -n "${2:-}" ]]; then printf -- '--- output ---\n%s\n--------------\n' "$2" >&2; fi
}

cleanup() {
  if [[ "$HOST_STARTED" == 1 ]]; then
    "$PLEXI" host stop >/dev/null 2>&1
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# ── Preconditions ────────────────────────────────────────────────────────────
if [[ ! -x "$PLEXI" ]]; then
  echo "FAIL: no executable at $PLEXI — run 'just build' first" >&2
  exit 1
fi
if [[ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
  echo "FAIL: neither DISPLAY nor WAYLAND_DISPLAY is set — no display to open a window on" >&2
  exit 1
fi
echo "smoke: binary=$PLEXI display=${DISPLAY:-$WAYLAND_DISPLAY}"

# ── 1. CLI answers at all ────────────────────────────────────────────────────
out="$("$PLEXI" --version 2>&1)"
if [[ $? -eq 0 && -n "$out" ]]; then ok "plexi --version -> $out"; else fail "plexi --version" "$out"; fi

# ── 2. doctor runs to completion ─────────────────────────────────────────────
# doctor reports problems by design, so the assertion is that it RUNS and
# produces a report — not that everything it checks is healthy.
out="$("$PLEXI" doctor 2>&1)"
if [[ -n "$out" ]]; then ok "plexi doctor produced a report"; else fail "plexi doctor" "$out"; fi

# ── 3. no host running yet ───────────────────────────────────────────────────
out="$("$PLEXI" host status --json 2>&1)"
if grep -q '"ready"[[:space:]]*:[[:space:]]*false' <<<"$out"; then
  ok "host status reports not ready"
else
  # A host left over from an earlier run is a dirty environment, not a pass.
  fail "host status should report not ready before start" "$out"
  "$PLEXI" host stop >/dev/null 2>&1
fi

# ── 4. host starts and opens a window ────────────────────────────────────────
out="$("$PLEXI" host start --ephemeral --timeout-secs 90 2>&1)"
if [[ $? -eq 0 ]]; then
  HOST_STARTED=1
  ok "host start confirmed readiness"
else
  fail "host start" "$out"
fi

# ── 5. the host agrees it is running ─────────────────────────────────────────
if [[ "$HOST_STARTED" == 1 ]]; then
  out="$("$PLEXI" host status --json 2>&1)"
  if grep -q '"ready"[[:space:]]*:[[:space:]]*true' <<<"$out"; then
    ok "host status reports ready: $(tr -d '\n' <<<"$out")"
  else
    fail "host status after start" "$out"
  fi

  # Pane and screenshot commands are pane-channel commands: they address the
  # host through PLEXI_SOCKET and refuse to guess. Point them at the host this
  # script just started, taken from its own status output rather than rebuilt
  # from a profile path.
  export PLEXI_SOCKET="$(sed -n 's/.*"socket"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' <<<"$out")"
  if [[ -S "$PLEXI_SOCKET" ]]; then
    ok "host socket is live at $PLEXI_SOCKET"
  else
    fail "could not resolve the host socket from status output" "$out"
  fi

  # ── 6. a pane spawns and the host serves pane IPC ──────────────────────────
  pane_id="$("$PLEXI" pane new "echo plexi-linux-smoke; sleep 600" -n smoke --no-focus 2>&1 | tr -d '[:space:]')"
  if [[ "$pane_id" =~ ^[0-9]+$ ]]; then
    ok "pane new -> id $pane_id"

    out="$("$PLEXI" pane list 2>&1)"
    if grep -q "\"id\"[[:space:]]*:[[:space:]]*$pane_id" <<<"$out"; then
      ok "pane list contains pane $pane_id"
    else
      fail "pane list" "$out"
    fi

    out="$("$PLEXI" pane capture "$pane_id" 2>&1)"
    if grep -q "plexi-linux-smoke" <<<"$out"; then
      ok "pane capture shows the command's own output (the PTY really ran)"
    else
      fail "pane capture did not echo the smoke marker" "$out"
    fi

    "$PLEXI" pane close "$pane_id" >/dev/null 2>&1
  else
    fail "pane new did not print a pane id" "$pane_id"
  fi

  # ── 7. the real render pipeline produces pixels ────────────────────────────
  shot="$WORK/boot.png"
  out="$("$PLEXI" host screenshot --output "$shot" 2>&1)"
  if [[ -s "$shot" ]]; then
    # A PNG that exists but is a uniform fill is the classic false pass, so
    # assert real size AND that the file is a PNG of non-trivial dimensions.
    size=$(stat -c %s "$shot")
    if [[ "$size" -gt 20000 ]]; then
      ok "host screenshot wrote $shot ($size bytes)"
    else
      fail "host screenshot is suspiciously small ($size bytes) — likely a blank surface" "$out"
    fi
  else
    fail "host screenshot produced no file" "$out"
  fi

  # ── 8. clean shutdown ──────────────────────────────────────────────────────
  out="$("$PLEXI" host stop 2>&1)"
  if [[ $? -eq 0 ]]; then
    HOST_STARTED=0
    ok "host stop"
  else
    fail "host stop" "$out"
  fi

  out="$("$PLEXI" host status --json 2>&1)"
  if grep -q '"ready"[[:space:]]*:[[:space:]]*false' <<<"$out"; then
    ok "host status reports not ready after stop"
  else
    fail "host status after stop" "$out"
  fi
fi

echo
if [[ "$FAILED" == 0 ]]; then
  echo "linux-smoke: PASS ($STEP_N steps)"
  exit 0
fi
echo "linux-smoke: FAIL" >&2
exit 1

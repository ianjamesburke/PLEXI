#!/usr/bin/env bash
# Installed-binary check: two chess panes. A bare `app call` names the
# ambiguity. `--pane` addresses the chosen instance and does not guess.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
export PLEXI_CHANNEL="dup-panes-e2e"
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
mkdir -p "$XDG_RUNTIME_DIR"

PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
SOCKET="$PROFILE/notify.sock"

start_host() {
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a "$BIN" >"$WORK/host.log" 2>&1 &
  else
    "$BIN" >"$WORK/host.log" 2>&1 &
  fi
  HOST_PID=$!
}

echo "starting host $BIN"
start_host
for _ in $(seq 1 90); do
  if [[ -S "$SOCKET" ]]; then
    break
  fi
  if ! kill -0 "$HOST_PID" 2>/dev/null; then
    echo "error: host exited before the socket appeared" >&2
    cat "$WORK/host.log" >&2 || true
    exit 1
  fi
  sleep 1
done
if [[ ! -S "$SOCKET" ]]; then
  echo "error: notify socket did not appear at $SOCKET" >&2
  cat "$WORK/host.log" >&2 || true
  exit 1
fi
export PLEXI_SOCKET="$SOCKET"

open_chess() {
  local label="$1"
  shift
  "$BIN" app open "$ROOT/apps/chess" "$@" >"$WORK/open-$label.log" 2>&1 || {
    echo "error: app open ($label) failed" >&2
    cat "$WORK/open-$label.log" >&2
    exit 1
  }
  local id
  id="$(awk '/^[0-9]+$/ { id=$0 } END { print id }' "$WORK/open-$label.log")"
  if [[ -z "$id" ]]; then
    echo "error: app open ($label) did not print a pane id" >&2
    cat "$WORK/open-$label.log" >&2
    exit 1
  fi
  echo "$id"
}

echo "opening first chess pane"
FIRST="$(open_chess first)"
echo "first chess pane $FIRST"

echo "opening second chess pane with --new"
SECOND="$(open_chess second --new)"
echo "second chess pane $SECOND"
if [[ "$FIRST" == "$SECOND" ]]; then
  echo "error: --new focused pane $FIRST instead of spawning another" >&2
  exit 1
fi

echo "a plain second open focuses the open board"
FOCUSED="$(open_chess again)"
if [[ "$FOCUSED" != "$FIRST" && "$FOCUSED" != "$SECOND" ]]; then
  echo "error: plain open spawned pane $FOCUSED instead of focusing an open board" >&2
  exit 1
fi

"$BIN" pane list >"$WORK/panes.txt"
TERM="$(python3 - "$WORK/panes.txt" <<'PY'
import json, sys
panes = json.load(open(sys.argv[1]))
terms = [pane for pane in panes if pane.get("type") == "terminal"]
if not terms:
    raise SystemExit("no terminal pane")
print(terms[0]["id"])
PY
)"
echo "terminal pane $TERM"

run_in_pane() {
  local outfile="$1"
  local cmd="$2"
  rm -f "$outfile"
  "$BIN" pane command "$TERM" "$cmd" --enter >"$WORK/enter.log" 2>&1
  for _ in $(seq 1 40); do
    if [[ -s "$outfile" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "error: pane command produced no output: $cmd" >&2
  cat "$WORK/enter.log" >&2 || true
  "$BIN" pane capture "$TERM" --lines 40 >"$WORK/capture.txt" 2>&1 || true
  cat "$WORK/capture.txt" >&2 || true
  return 1
}

echo "waiting until both boards expose chess.state"
ready=0
for _ in $(seq 1 90); do
  if run_in_pane "$WORK/bare.out" "$BIN app call chess chess.state --json > '$WORK/bare.out'"; then
    if python3 - "$WORK/bare.out" "$FIRST" "$SECOND" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
start = text.find("{")
if start < 0:
    raise SystemExit(1)
reply = json.loads(text[start:])
if reply.get("error_code") != "ambiguous_instance":
    raise SystemExit(1)
panes = {int(p) for p in reply.get("panes", [])}
want = {int(sys.argv[2]), int(sys.argv[3])}
if panes != want:
    raise SystemExit(f"panes {panes} != {want}")
print("ambiguous_instance", sorted(panes))
PY
    then
      ready=1
      break
    fi
  fi
  sleep 2
done
if [[ "$ready" != 1 ]]; then
  echo "error: bare app call did not return ambiguous_instance for both panes" >&2
  cat "$WORK/bare.out" >&2 || true
  tail -n 80 "$WORK/host.log" >&2 || true
  exit 1
fi

address_one() {
  local pane="$1"
  local outfile="$WORK/pane-$pane.out"
  run_in_pane "$outfile" "$BIN app call chess chess.state --json --pane $pane > '$outfile'"
  python3 - "$outfile" "$pane" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
start = text.find("{")
reply = json.loads(text[start:])
code = reply.get("error_code")
if code == "ambiguous_instance":
    raise SystemExit(f"pane {sys.argv[2]} was still ambiguous: {reply}")
if code not in ("permission_required", None) and reply.get("ok") is not True:
    raise SystemExit(f"pane {sys.argv[2]} unexpected reply: {reply}")
print("addressed", sys.argv[2], code or "ok")
PY
}

echo "addressing each pane"
address_one "$FIRST"
address_one "$SECOND"

echo "confirming the pending tool names the chosen pane"
"$BIN" assistant permission list >"$WORK/pending.json"
python3 - "$WORK/pending.json" "$FIRST" "$SECOND" <<'PY'
import json, sys
text = open(sys.argv[1]).read()
start = text.find("{")
body = json.loads(text[start:])
tools = [row.get("tool", "") for row in body.get("pending", [])]
first, second = sys.argv[2], sys.argv[3]
need = (f"chess:{first}__chess.state", f"chess:{second}__chess.state")
missing = [name for name in need if name not in tools]
if missing:
    raise SystemExit(f"pending tools {tools} missing {missing}")
print("pending", need)
PY

if ! grep -q "ambiguous_instance" "$WORK/host.log" "$PROFILE/plexi.log"; then
  echo "error: host log has no ambiguous_instance trace" >&2
  exit 1
fi

echo "duplicate app panes: pass"

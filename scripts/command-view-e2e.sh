#!/usr/bin/env bash
# Installed-binary check for the command view.
# Contract: src/host/command_view.rs
# Profile: resolve_channel_dir in src/config/mod.rs. A channel-named binary
# (plexi-pr-2699) uses ~/.plexi-pr-2699 and ignores PLEXI_CHANNEL.
#
#   bash scripts/command-view-e2e.sh [path-to-plexi-binary]
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${1:-${PLEXI_BIN:-$ROOT/target/release/plexi}}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN" >&2
  exit 1
fi
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

WORK="$(mktemp -d -t plexi-command-view-XXXXXX)"
HOST_PID=""
FOLLOW_PID=""
PASS_N=0
FAIL_N=0

pass() { PASS_N=$((PASS_N + 1)); printf 'PASS: %s\n' "$1"; }
fail() { FAIL_N=$((FAIL_N + 1)); printf 'FAIL: %s\n' "$1" >&2; }

cleanup() {
  if [[ -n "$FOLLOW_PID" ]]; then
    kill "$FOLLOW_PID" 2>/dev/null || true
    wait "$FOLLOW_PID" 2>/dev/null || true
  fi
  if [[ -n "$HOST_PID" ]]; then
    kill "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi" "$WORK/workspace"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-$(id -u)}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cd "$WORK/workspace"

# Same rule as resolve_channel_dir: plexi-pr-2699 -> .plexi-pr-2699, and that
# name wins over PLEXI_CHANNEL. Only a bare plexi binary adopts the env channel.
bin_base="$(basename "$BIN")"
bin_base="${bin_base%.exe}"
bin_base="${bin_base%.EXE}"
if [[ "$bin_base" == plexi-* && -n "${bin_base#plexi-}" ]]; then
  unset PLEXI_CHANNEL || true
  PROFILE="$HOME/.plexi-${bin_base#plexi-}"
else
  export PLEXI_CHANNEL="command-view-e2e"
  PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
fi
SOCKET="$PROFILE/notify.sock"
echo "profile $PROFILE (binary $bin_base)"

start_host() {
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a -s "-screen 0 1600x1000x24" "$BIN" >"$WORK/host.log" 2>&1 &
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

echo "opening command view"
"$BIN" app open command-view >"$WORK/open.log" 2>&1 || true
PANE=""
for _ in $(seq 1 30); do
  if "$BIN" pane list >"$WORK/panes.txt" 2>"$WORK/panes.err"; then
    PANE="$(python3 - "$WORK/panes.txt" <<'PY'
import json, sys
try:
    panes = json.load(open(sys.argv[1]))
except Exception:
    raise SystemExit(0)
for pane in panes:
    if pane.get("manifest_id") == "command-view" or pane.get("title") == "Command":
        print(pane.get("id", ""))
        break
PY
)"
    if [[ -n "$PANE" ]]; then
      break
    fi
  fi
  sleep 1
done
if [[ -z "$PANE" ]]; then
  fail "command-view pane did not open"
  cat "$WORK/open.log" "$WORK/panes.txt" "$WORK/host.log" >&2 || true
else
  pass "command-view pane $PANE is open"
fi

cv() {
  "$BIN" command-view "$@"
}

echo "ungranted send"
set +e
UNGRANTED="$(cv send --lead lead-a --text "editing notes" 2>"$WORK/ungranted.err")"
UNGRANTED_CODE=$?
set -e
printf '%s\n' "$UNGRANTED" >"$WORK/ungranted.json"
if [[ "$UNGRANTED_CODE" == 2 ]] && python3 - "$WORK/ungranted.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
raise SystemExit(0 if body.get("error_code") == "permission_required" and body.get("pending_request_id") and not body.get("leads") else 1)
PY
then
  pass "ungranted send exits 2 and does not create a lead"
else
  fail "ungranted send (exit $UNGRANTED_CODE)"
  cat "$WORK/ungranted.json" "$WORK/ungranted.err" >&2 || true
fi

echo "grant and send"
cv allow --tool command.send --lead lead-a --text "editing notes" >"$WORK/allow-send.json"
cv send --lead lead-a --text "editing notes" >"$WORK/send.json"
RUN_A="$(python3 - "$WORK/send.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("ok") is not True:
    raise SystemExit(f"send failed: {body}")
lead = next(item for item in body["leads"] if item["id"] == "lead-a")
if lead["status"] != "running" or lead["last_output"] != "editing notes":
    raise SystemExit(f"lead not running: {lead}")
print(lead["runs"][0]["id"])
PY
)" || { fail "send did not mark lead-a running"; RUN_A=""; }
if [[ -n "$RUN_A" ]]; then
  pass "send marks lead-a running with last output"
fi

echo "enqueue both leads"
cv allow --tool command.enqueue --lead lead-a --text "ship the notes" >"$WORK/allow-enq-a.json"
cv enqueue --lead lead-a --text "ship the notes" >"$WORK/enq-a.json"
cv allow --tool command.enqueue --lead lead-b --text "wait here" >"$WORK/allow-enq-b.json"
cv enqueue --lead lead-b --text "wait here" >"$WORK/enq-b.json"
python3 - "$WORK/enq-b.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
ids = {item["id"]: item for item in body["leads"]}
a, b = ids["lead-a"], ids["lead-b"]
if a["status"] != "running" or a["queue"][0]["status"] != "pending" or a["queue"][0]["text"] != "ship the notes":
    raise SystemExit(f"lead-a queue: {a}")
if b["status"] != "idle" or b["last_output"] != "" or b["queue"][0]["text"] != "wait here":
    raise SystemExit(f"lead-b: {b}")
PY
if [[ $? -eq 0 ]]; then
  pass "projection lists both leads, queue, and last output"
else
  fail "projection after enqueue"
  cat "$WORK/enq-b.json" >&2 || true
fi

if [[ -n "$PANE" ]]; then
  "$BIN" pane state "$PANE" >"$WORK/pane-state.json" 2>"$WORK/pane-state.err" || true
  if python3 - "$WORK/pane-state.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
state = body.get("app_state") or {}
ids = {item["id"] for item in state.get("leads", [])}
raise SystemExit(0 if {"lead-a", "lead-b"} <= ids else 1)
PY
  then
    pass "pane state shows the same leads"
  else
    fail "pane state app_state"
    cat "$WORK/pane-state.json" "$WORK/pane-state.err" >&2 || true
  fi
fi

echo "pause and cancel"
cv allow --tool command.pause --run "$RUN_A" >"$WORK/allow-pause.json"
cv pause --run "$RUN_A" >"$WORK/pause.json"
python3 - "$WORK/pause.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
lead = next(item for item in body["leads"] if item["id"] == "lead-a")
raise SystemExit(0 if lead["status"] == "idle" else 1)
PY
if [[ $? -eq 0 ]]; then pass "pause marks the run idle"; else fail "pause"; cat "$WORK/pause.json" >&2 || true; fi

cv allow --tool command.cancel --run "$RUN_A" >"$WORK/allow-cancel.json"
cv cancel --run "$RUN_A" >"$WORK/cancel.json"
python3 - "$WORK/cancel.json" "$RUN_A" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
lead = next(item for item in body["leads"] if item["id"] == "lead-a")
run = next(item for item in lead["runs"] if item["id"] == sys.argv[2])
raise SystemExit(0 if run["status"] == "done" and lead["status"] == "done" else 1)
PY
if [[ $? -eq 0 ]]; then pass "cancel marks the run done"; else fail "cancel"; cat "$WORK/cancel.json" >&2 || true; fi

echo "block and resolve"
cv block --lead lead-b --run run-9 --summary "which file" >"$WORK/block.json"
NY="$(python3 - "$WORK/block.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
lead = next(item for item in body["leads"] if item["id"] == "lead-b")
if lead["status"] != "waiting on you" or not lead["needs_you"]:
    raise SystemExit(f"not waiting: {lead}")
print(lead["needs_you"][0]["id"])
PY
)" || NY=""
if [[ -n "$NY" ]]; then
  pass "needs you is inline on lead-b"
else
  fail "block did not surface needs you"
  cat "$WORK/block.json" >&2 || true
fi

echo "command pane pixels"
SHOT="$WORK/command-pane.png"
SHOT_OK=0
if [[ -n "$PANE" ]]; then
  sleep 1
  set +e
  if command -v timeout >/dev/null 2>&1; then
    timeout 25 "$BIN" host screenshot --pane "$PANE" --output "$SHOT" >"$WORK/shot.log" 2>&1
  else
    "$BIN" host screenshot --pane "$PANE" --output "$SHOT" >"$WORK/shot.log" 2>&1
  fi
  SHOT_CODE=$?
  set -e
  if [[ "$SHOT_CODE" == 0 ]] && python3 - "$SHOT" <<'PY'
import struct, sys, zlib
data = open(sys.argv[1], "rb").read()
if data[:8] != b"\x89PNG\r\n\x1a\n":
    raise SystemExit("not a png")
pos = 8
width = height = bit_depth = color_type = interlace = None
idat = []
while pos + 8 <= len(data):
    length = struct.unpack(">I", data[pos:pos + 4])[0]
    kind = data[pos + 4:pos + 8]
    chunk = data[pos + 8:pos + 8 + length]
    pos += 12 + length
    if kind == b"IHDR":
        width, height, bit_depth, color_type, _, _, interlace = struct.unpack(">IIBBBBB", chunk)
    elif kind == b"IDAT":
        idat.append(chunk)
    elif kind == b"IEND":
        break
if not width or width < 200 or height < 80:
    raise SystemExit(f"pane crop too small: {width}x{height}")
if bit_depth != 8 or interlace != 0 or color_type not in (2, 6):
    raise SystemExit(0)
channels = 3 if color_type == 2 else 4
raw = zlib.decompress(b"".join(idat))
stride = width * channels
i = 0
prev = bytearray(stride)
samples = []
step_y = max(1, height // 40)
step_x = max(1, width // 40)
for y in range(height):
    filt = raw[i]
    i += 1
    row = bytearray(raw[i:i + stride])
    i += stride
    if filt == 1:
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            row[x] = (row[x] + left) & 255
    elif filt == 2:
        for x in range(stride):
            row[x] = (row[x] + prev[x]) & 255
    elif filt == 3:
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            row[x] = (row[x] + ((left + prev[x]) // 2)) & 255
    elif filt == 4:
        for x in range(stride):
            a = row[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            pr = a if pa <= pb and pa <= pc else b if pb <= pc else c
            row[x] = (row[x] + pr) & 255
    elif filt != 0:
        raise SystemExit(f"bad filter {filt}")
    if y % step_y == 0:
        for x in range(0, width, step_x):
            samples.append(row[x * channels])
    prev = row
mean = sum(samples) / len(samples)
var = sum((sample - mean) ** 2 for sample in samples) / len(samples)
if var < 20:
    raise SystemExit(f"flat image variance {var:.1f}")
print(f"{width}x{height} variance {var:.0f}")
PY
  then
    SHOT_OK=1
  fi
  if command -v tesseract >/dev/null 2>&1 && [[ -s "$SHOT" ]]; then
    tesseract "$SHOT" stdout --psm 6 >"$WORK/shot.txt" 2>"$WORK/shot.ocr" || true
    if ! python3 - "$WORK/shot.txt" <<'PY'
import sys
text = open(sys.argv[1], encoding="utf-8", errors="replace").read().lower()
missing = [word for word in ("lead-a", "lead-b", "waiting") if word not in text]
if missing:
    raise SystemExit("ocr missed " + ", ".join(missing))
PY
    then
      SHOT_OK=0
      echo "ocr did not read both leads and the waiting state" >&2
      cat "$WORK/shot.txt" >&2 || true
    fi
  fi
fi
if [[ -f "$SHOT" ]]; then
  cp "$SHOT" /tmp/plexi-command-view-pane.png
fi
if [[ "$SHOT_OK" == 1 ]]; then
  pass "command pane shows two leads, one waiting on you"
else
  fail "command pane render"
  cat "$WORK/shot.log" >&2 || true
fi

cv resolve "$NY" --approve >"$WORK/resolve.json"
python3 - "$WORK/resolve.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
lead = next(item for item in body["leads"] if item["id"] == "lead-b")
run = next(item for item in lead["runs"] if item["id"] == "run-9")
if body.get("resolution") != "approved" or run["status"] != "running" or lead["status"] != "running":
    raise SystemExit(f"not unblocked: {body}")
PY
if [[ $? -eq 0 ]]; then pass "resolve unblocks the run"; else fail "resolve"; cat "$WORK/resolve.json" >&2 || true; fi

echo "follow"
: >"$WORK/follow.ndjson"
cv --follow >"$WORK/follow.ndjson" 2>"$WORK/follow.err" &
FOLLOW_PID=$!
sleep 1
BEFORE="$(wc -l <"$WORK/follow.ndjson" | tr -d ' ')"
cv allow --tool command.enqueue --lead lead-c --text "live" >/dev/null
cv enqueue --lead lead-c --text "live" >"$WORK/live.json"
SEEN=0
for _ in $(seq 1 20); do
  if python3 - "$WORK/follow.ndjson" "$BEFORE" <<'PY'
import json, sys
lines = [line for line in open(sys.argv[1]) if line.strip()]
start = int(sys.argv[2])
for line in lines[start:]:
    body = json.loads(line)
    if body.get("event") == "command.view" and any(item.get("id") == "lead-c" for item in body.get("leads", [])):
        raise SystemExit(0)
raise SystemExit(1)
PY
  then
    SEEN=1
    break
  fi
  sleep 0.5
done
kill "$FOLLOW_PID" 2>/dev/null || true
wait "$FOLLOW_PID" 2>/dev/null || true
FOLLOW_PID=""
if [[ "$SEEN" == 1 ]]; then
  pass "follow received a command.view event without another list"
else
  fail "follow did not receive the enqueue"
  cat "$WORK/follow.ndjson" "$WORK/follow.err" >&2 || true
fi

echo
printf '%s PASS, %s FAIL\n' "$PASS_N" "$FAIL_N"
if [[ "$FAIL_N" != 0 ]]; then
  exit 1
fi

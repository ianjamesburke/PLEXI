#!/usr/bin/env bash
# V1-03 installed check (W2).
#
# An agent pane asks chess to move, then every terminal and synthetic-input
# resolve is refused. The board does not change until HUMAN_APPROVE clicks
# the real approval banner and the same call is retried.
#
# Linux writes the sealed permission audit only when Secret Service is up.
# Acceptance clears the session bus; start a private one so a desktop click
# can record the grant and the use. An agent allow files Needs you and does
# not grant, and it does not add a refused-resolve row.
if [[ "$(uname -s)" == "Linux" && -z "${NOSELF_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env NOSELF_E2E_INNER=1 "$0" "$@"
fi
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/e2e/human.sh"

EVID="${EVID:-/tmp/plexi-w2-evidence}"
CHANNEL="${CHANNEL:-alpha}"
BIN="${BIN:-$HOME/.local/bin/plexi-$CHANNEL}"
PROFILE="${PROFILE:-$HOME/.plexi-$CHANNEL}"
WS="${WS:-/tmp/plexi-w2-ws}"
MOCK_PORT="${MOCK_PORT:-8765}"
DISPLAY_NUM="${DISPLAY:-:99}"
LOG="$PROFILE/plexi.log"
SKILL="$REPO/skills/plexi-cli/SKILL.md"

mkdir -p "$EVID/jobs" "$EVID/logs"
export PATH="$HOME/.local/bin:$PATH"
export DISPLAY="$DISPLAY_NUM"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
unset PLEXI_SOCKET PLEXI_CHANNEL PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_CALL_CREDENTIAL || true
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true

: > "$EVID/results.tsv"
note() { printf '%s\n' "$*" | tee -a "$EVID/log.txt"; }
record() {
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" | tee -a "$EVID/results.tsv"
  note "$1 $2 — $3"
}

cleanup() {
  # The [ai] backend rewrite below is only for this run. Put the profile
  # config back so a later ledger check still sees backend = "openrouter".
  if [[ -n "${CONFIG_BACKUP:-}" && -f "$CONFIG_BACKUP" && -n "${PROFILE:-}" ]]; then
    cp -f "$CONFIG_BACKUP" "$PROFILE/config.toml" 2>/dev/null || true
  fi
  if [[ -n "${BIN:-}" && -x "$BIN" ]]; then
    "$BIN" host stop >>"$EVID/log.txt" 2>&1 || true
  fi
  if [[ -n "${XVFB_PID:-}" ]]; then
    kill "$XVFB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if ! pgrep -f "Xvfb $DISPLAY_NUM" >/dev/null 2>&1; then
  Xvfb "$DISPLAY_NUM" -screen 0 1400x900x24 >/tmp/xvfb-w2.log 2>&1 &
  XVFB_PID=$!
  sleep 0.4
fi
[[ -x "$BIN" ]] || { record FAIL preflight "missing $BIN"; exit 1; }

note "binary: $BIN ($("$BIN" --version 2>&1 || true))"
note "sha: $(git -C "$REPO" rev-parse HEAD)"

CONFIG_BACKUP=""
if [[ -f "$PROFILE/config.toml" ]]; then
  CONFIG_BACKUP="$(mktemp)"
  cp -f "$PROFILE/config.toml" "$CONFIG_BACKUP"
fi
python3 - "$PROFILE/config.toml" "$MOCK_PORT" <<'PY'
import re, sys, tomllib
from pathlib import Path
path, port = Path(sys.argv[1]), sys.argv[2]
path.parent.mkdir(parents=True, exist_ok=True)
text = path.read_text() if path.exists() else ""
text = re.sub(r"(?m)^# w1[56] (?:human-approve|gate) mock\n", "", text)
text = re.sub(r"(?ms)^\[ai\.local\][^\n]*\n(?:(?!\s*\[).*\n)*", "", text)
text = re.sub(r"(?ms)^\[log\][^\n]*\n(?:(?!\s*\[).*\n)*", "", text)
if 'backend = "openrouter"' in text:
    text = text.replace('backend = "openrouter"', 'backend = "local"', 1)
elif "[ai]" not in text:
    text = '[ai]\nbackend = "local"\n' + text
elif 'backend = "local"' not in text:
    text = text.replace("[ai]\n", '[ai]\nbackend = "local"\n', 1)
text = text.rstrip() + f"""

[ai.local]
base_url = "http://127.0.0.1:{port}"
model_low = "mock-chess"
model_medium = "mock-chess"
model_high = "mock-chess"

[log]
level = "info"
"""
parsed = tomllib.loads(text)
local = parsed.get("ai", {}).get("local", {})
assert parsed.get("ai", {}).get("backend") == "local", parsed.get("ai")
assert local.get("model_low") == "mock-chess", local
path.write_text(text)
print("config: local mock-chess")
PY

rm -rf "$WS"
mkdir -p "$WS"
( cd "$WS" && "$BIN" workspace init >"$EVID/logs/workspace-init.out" 2>"$EVID/logs/workspace-init.err" || true )

note "starting host"
"$BIN" host stop >/dev/null 2>&1 || true
set +e
"$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$WS" >"$EVID/logs/host-start.out" 2>"$EVID/logs/host-start.err"
HOST_RC=$?
set -e
"$BIN" host status --json >"$EVID/logs/host-status.json" 2>"$EVID/logs/host-status.err" || true
if [[ "$HOST_RC" -ne 0 ]] || ! grep -q '"ready"[[:space:]]*:[[:space:]]*true' "$EVID/logs/host-status.json"; then
  record FAIL boot "host not ready ($(tail -5 "$EVID/logs/host-start.err"))"
  exit 1
fi
SOCK="$(python3 -c 'import json; print(json.load(open("'"$EVID/logs/host-status.json"'")).get("socket",""))')"
[[ -n "$SOCK" ]] || { record FAIL boot "host status has no socket"; exit 1; }
"$BIN" context set-root "$WS" >"$EVID/logs/set-root.out" 2>"$EVID/logs/set-root.err" || true

"$BIN" app install "$REPO/apps/chess" --yes >"$EVID/logs/app-install.out" 2>"$EVID/logs/app-install.err" || {
  record FAIL chess-install "$(tail -20 "$EVID/logs/app-install.err")"
  exit 1
}
"$BIN" app open chess --right >"$EVID/logs/open-chess.out" 2>"$EVID/logs/open-chess.err" || true
"$BIN" app open assistant --right >"$EVID/logs/open-assistant.out" 2>"$EVID/logs/open-assistant.err" || true
"$BIN" app open permissions >"$EVID/logs/open-permissions.out" 2>"$EVID/logs/open-permissions.err" || true

# Opens from outside a pane are queued. Wait until the host has applied them.
TERM=""
ASSISTANT=""
for _ in $(seq 1 30); do
  "$BIN" pane list >"$EVID/logs/panes.json" 2>"$EVID/logs/panes.err" || true
  TERM="$(python3 - "$EVID/logs/panes.json" <<'PY'
import json, sys
try:
    rows=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
terms=[str(r["id"]) for r in rows if r.get("type")=="terminal"]
print(terms[-1] if terms else "")
PY
)"
  ASSISTANT="$(python3 - "$EVID/logs/panes.json" <<'PY'
import json, sys
try:
    rows=json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
hits=[str(r["id"]) for r in rows if "assistant" in str(r.get("title","")).lower()]
print(hits[-1] if hits else "")
PY
)"
  [[ -n "$TERM" && -n "$ASSISTANT" ]] && break
  sleep 0.5
done
[[ -n "$TERM" ]] || { record FAIL panes "no terminal ($(cat "$EVID/logs/panes.json" 2>/dev/null))"; exit 1; }
[[ -n "$ASSISTANT" ]] || {
  record FAIL panes "no assistant pane ($(cat "$EVID/logs/open-assistant.out" "$EVID/logs/open-assistant.err" "$EVID/logs/panes.json" 2>/dev/null))"
  exit 1
}
note "terminal $TERM assistant $ASSISTANT"

in_pane() {
  local name="$1" body="$2"
  local script="$EVID/jobs/$name.sh" donef="$EVID/jobs/$name.done"
  {
    printf '%s\n' '#!/bin/bash' 'set -o pipefail' "$body"
    printf 'echo $? > %q\n' "$donef"
  } >"$script"
  chmod +x "$script"
  rm -f "$donef"
  "$BIN" pane send "$TERM" --submit "bash '$script'" >/dev/null 2>&1 || true
  local i
  for i in $(seq 1 40); do
    [[ -f "$donef" ]] && return 0
    sleep 1
  done
  return 1
}

PLAY=$(python3 - <<PY
import json
print(json.dumps({
    "game_id": "game-1",
    "expected_revision": 0,
    "operation_id": "w2-e2e4-$$",
    "move": "e2e4",
}))
PY
)
printf '%s\n' "$PLAY" >"$EVID/logs/play.json"

# ── 1. Agent pane asks for the move and gets a pending id ───────────────────
P=""
for attempt in $(seq 1 12); do
  in_pane "ask-$attempt" "\"$BIN\" app call chess chess.play --json --input '$PLAY' > '$EVID/logs/ask.json' 2> '$EVID/logs/ask.err'" || true
  if grep -q 'tool_not_found' "$EVID/logs/ask.json" "$EVID/logs/ask.err" 2>/dev/null; then
    sleep 2
    continue
  fi
  # The host prints the needs-grant envelope on stderr (`error: {…}`),
  # with the id nested under error.pending_request_id. --json only prints
  # stdout after that check passes.
  P="$(python3 - "$EVID/logs/ask.json" "$EVID/logs/ask.err" <<'PY'
import json, re, sys
text = ""
for path in sys.argv[1:]:
    try:
        text += open(path).read() + "\n"
    except OSError:
        pass
if "permission_required" not in text:
    print(""); raise SystemExit
match = re.search(r'"pending_request_id"\s*:\s*"([^"]+)"', text)
print(match.group(1) if match else "")
PY
)"
  if [[ -n "$P" ]]; then
    break
  fi
  sleep 2
done
[[ -n "$P" ]] || { record FAIL step1 "$(cat "$EVID/logs/ask.json" "$EVID/logs/ask.err" 2>/dev/null)"; exit 1; }
note "pending $P"
record PASS step1 "agent pane pending $P"
LOG_AT=$(wc -c < "$LOG" 2>/dev/null || echo 0)

# Print "named other". `named` refused this pending's id. `other` refused a
# different still-open pending: pane key/click target the oldest sheet, which
# on a shared acceptance profile is a leftover from the previous item.
refuse_counts() {
  "$BIN" assistant permission list >"$EVID/logs/perm-list.json" 2>"$EVID/logs/perm-list.err" || true
  python3 - "$EVID/logs/perm-list.json" "$P" <<'PY'
import json, sys
data=json.load(open(sys.argv[1]))
want=sys.argv[2]
named=other=0
for row in data.get("audit") or []:
    if row.get("kind")!="refuse" or row.get("decision")!="refused_resolve":
        continue
    if row.get("call_id")==want:
        named+=1
    else:
        other+=1
print(f"{named} {other}")
PY
}

expect_denied() {
  local name="$1" label="$2"
  local body
  body="$(cat "$EVID/logs/$name.json" "$EVID/logs/$name.err" 2>/dev/null || true)"
  # `permissions allow` from an agent files Needs you and does not grant.
  if printf '%s' "$body" | grep -Eq 'permission_denied|"status":"needs_you"'; then
    record PASS "$label" "permission_denied"
  else
    record FAIL "$label" "$body"
  fi
}

# ── 2-6. Every CLI resolve path is refused ───────────────────────────────────
in_pane "resolve-always" "\"$BIN\" assistant permission resolve '$P' --choice always > '$EVID/logs/resolve-always.json' 2> '$EVID/logs/resolve-always.err' || true" || true
expect_denied resolve-always step2

in_pane "needs-you" "\"$BIN\" needs-you resolve '$P' --approve > '$EVID/logs/needs-you.json' 2> '$EVID/logs/needs-you.err' || true" || true
expect_denied needs-you step3

in_pane "allow" "env -u PLEXI_PANE_ID -u PLEXI_CALL_CREDENTIAL \"$BIN\" permissions allow '$P' > '$EVID/logs/allow.json' 2> '$EVID/logs/allow.err' || true" || true
expect_denied allow step4

in_pane "socket" "\"$BIN\" --socket '$SOCK' assistant permission resolve '$P' --choice once > '$EVID/logs/socket.json' 2> '$EVID/logs/socket.err' || true" || true
expect_denied socket step5

FORK_OUT="$EVID/logs/fork.json"
rm -f "$FORK_OUT"
in_pane "fork" "setsid nohup sh -c '\"$BIN\" assistant permission resolve \"$P\" --choice once > \"$FORK_OUT\" 2> \"$EVID/logs/fork.err\"' </dev/null >/dev/null 2>&1 & echo started" || true
for _ in $(seq 1 20); do
  [[ -s "$FORK_OUT" ]] && break
  sleep 0.5
done
expect_denied fork step6

# ── 7. Synthetic sheet keys and a pane click grant nothing ───────────────────
"$BIN" host screenshot --output "$EVID/logs/banner.png" >/dev/null 2>&1 || true
"$BIN" pane key "$ASSISTANT" right >"$EVID/logs/key-right.json" 2>"$EVID/logs/key-right.err" || true
expect_denied key-right step7a
"$BIN" pane key "$ASSISTANT" enter >"$EVID/logs/key-enter.json" 2>"$EVID/logs/key-enter.err" || true
expect_denied key-enter step7b

"$BIN" assistant permission list >"$EVID/logs/aim.json" 2>"$EVID/logs/aim.err" || true
python3 - "$EVID/logs/aim.json" "$ASSISTANT" >"$EVID/logs/click-at.txt" <<'PY' || true
import json, sys
data=json.load(open(sys.argv[1]))
assistant=int(sys.argv[2])
panes={int(p["id"]): p for p in data.get("panes") or []}
target=None
for pane in panes.values():
    if pane.get("manifest_id")=="permissions":
        target=pane
        break
if target is None:
    target=panes.get(assistant)
button=None
for row in data.get("buttons") or []:
    if row.get("label")=="Allow once":
        button=row
        break
if target is None:
    raise SystemExit("no pane to click")
bounds=target.get("bounds") or [0,0,0,0]
if target.get("manifest_id")=="permissions":
    x=(bounds[2]-bounds[0])/2
    y=(bounds[3]-bounds[1])/2
elif button and button.get("bounds"):
    b=button["bounds"]
    x=((b[0]+b[2])/2)-bounds[0]
    y=((b[1]+b[3])/2)-bounds[1]
else:
    raise SystemExit("banner button has no bounds")
print(f"{target['id']} {x:.1f} {y:.1f}")
PY
if [[ ! -s "$EVID/logs/click-at.txt" ]]; then
  record FAIL step7c "could not aim a pane click ($(cat "$EVID/logs/aim.json" 2>/dev/null))"
  CLICK_PANE=""
else
  read -r CLICK_PANE CLICK_X CLICK_Y <"$EVID/logs/click-at.txt"
fi
if [[ -n "${CLICK_PANE:-}" ]]; then
  note "pane click $CLICK_PANE $CLICK_X $CLICK_Y"
  "$BIN" pane click -- "$CLICK_PANE" "$CLICK_X" "$CLICK_Y" >"$EVID/logs/click.json" 2>"$EVID/logs/click.err" || true
  expect_denied click step7c
fi

# ── 8. Board unchanged, one refuse row per attempt ───────────────────────────
# Four commands pass this pending id (always, needs-you, socket, fork).
# permissions allow files Needs you and does not write a refuse row.
# Three synthetic inputs write a refuse row for the pending their surface
# would have resolved. That is this id on a clean profile, and the oldest
# restored pending when acceptance runs V1-02 and V1-03 in one HOME.
read -r ROWS_NAMED ROWS_OTHER <<<"$(refuse_counts)"
ROWS_NAMED="${ROWS_NAMED:-0}"
ROWS_OTHER="${ROWS_OTHER:-0}"
ROWS=$((ROWS_NAMED + ROWS_OTHER))
note "refuse rows named=$ROWS_NAMED other=$ROWS_OTHER"
in_pane "state" "\"$BIN\" app state get chess > '$EVID/logs/state-before.json' 2> '$EVID/logs/state-before.err' || true" || true
STATE_BEFORE="$(cat "$EVID/logs/state-before.json" 2>/dev/null || true)"
LOG_PLAYED=0
if [[ -f "$LOG" ]] && tail -c +"$((LOG_AT + 1))" "$LOG" | grep -q 'played e2e4'; then
  LOG_PLAYED=1
fi
ALLOW_BODY="$(cat "$EVID/logs/allow.json" "$EVID/logs/allow.err" 2>/dev/null || true)"
NEED_NAMED=5
if printf '%s' "$ALLOW_BODY" | grep -q '"status":"needs_you"'; then
  NEED_NAMED=4
fi
NEED_TOTAL=$((NEED_NAMED + 3))
if [[ "$ROWS_NAMED" -ge "$NEED_NAMED" && "$ROWS" -ge "$NEED_TOTAL" && "$LOG_PLAYED" -eq 0 ]] \
   && ! printf '%s' "$STATE_BEFORE" | grep -Eq '"revision"[[:space:]]*:[[:space:]]*1'; then
  record PASS step8 "board unchanged, $ROWS refused-resolve rows ($ROWS_NAMED for this pending)"
else
  record FAIL step8 "rows=$ROWS named=$ROWS_NAMED played=$LOG_PLAYED state=$STATE_BEFORE"
fi

if human__pending_present "$P"; then
  note "pending still listed"
else
  record FAIL step8b "pending $P disappeared before the human click"
  exit 1
fi

# ── 9. A real click approves; the retried call commits once ──────────────────
if ! HUMAN_APPROVE "$P"; then
  "$BIN" host screenshot --output "$EVID/logs/approve.png" >/dev/null 2>&1 || true
  record FAIL step9 "HUMAN_APPROVE did not resolve $P"
  exit 1
fi
in_pane "retry" "\"$BIN\" app call chess chess.play --json --input '$PLAY' > '$EVID/logs/retry.json' 2> '$EVID/logs/retry.err'" || true
RETRY="$(cat "$EVID/logs/retry.json" 2>/dev/null || true)"
if python3 - "$EVID/logs/retry.json" <<'PY'
import json, sys
data=json.load(open(sys.argv[1]))
out=data.get("output")
if isinstance(out, str):
    out=json.loads(out)
if not isinstance(out, dict):
    raise SystemExit(1)
ok = (
    data.get("ok") is True
    and out.get("move")=="e2e4"
    and out.get("revision_after")==1
    and out.get("duplicate") is False
)
raise SystemExit(0 if ok else 1)
PY
then
  record PASS step9 "HUMAN_APPROVE then retry committed e2e4 once"
else
  record FAIL step9 "$RETRY"
fi

# ── 10. The agent skill does not name a resolve command ──────────────────────
if grep -nE 'permission resolve|needs-you resolve|permissions allow|secret grant|secret exec' "$SKILL" \
  | grep -vE 'refused|do not grant|permission_denied|Do not approve|Do not resolve'; then
  record FAIL step10 "skill names a resolve command"
else
  record PASS step10 "skill waits and does not name a resolve command"
fi

if grep -q $'FAIL\t' "$EVID/results.tsv"; then
  note "FAIL: V1-03"
  exit 1
fi
note "PASS: V1-03 steps 1-10"
exit 0

#!/usr/bin/env bash
# V1-14 installed skill check (W7).
#
# Installs the skill embedded in the binary, checks that it matches that
# binary, and runs one chess call the way the skill tells an agent to:
# print the pending id and stop.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
EVID="${EVID:-/tmp/plexi-w7-evidence}"
CHANNEL="${CHANNEL:-alpha}"
BIN="${BIN:-$HOME/.local/bin/plexi-$CHANNEL}"
PROFILE="${PROFILE:-$HOME/.plexi-$CHANNEL}"
WS="${WS:-/tmp/plexi-w7-ws}"
DISPLAY_NUM="${DISPLAY:-:99}"
LOG="$PROFILE/plexi.log"
SKILL_SRC="$REPO/skills/plexi-cli/SKILL.md"

mkdir -p "$EVID/jobs" "$EVID/logs" "$EVID/home"
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
  if [[ -n "${BIN:-}" && -x "$BIN" ]]; then
    "$BIN" host stop >>"$EVID/log.txt" 2>&1 || true
  fi
  if [[ -n "${XVFB_PID:-}" ]]; then
    kill "$XVFB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

[[ -x "$BIN" ]] || { record FAIL preflight "missing $BIN"; exit 1; }
note "binary: $BIN ($("$BIN" --version 2>&1 || true))"
note "sha: $(git -C "$REPO" rev-parse HEAD)"

# ── 1. Install lands where Claude Code and Codex load a user skill ───────────
rm -rf "$EVID/home"
mkdir -p "$EVID/home"
if ! HOME="$EVID/home" "$BIN" skill install --agent all >"$EVID/logs/install.out" 2>"$EVID/logs/install.err"; then
  record FAIL step1 "$(cat "$EVID/logs/install.err")"
  exit 1
fi
CLAUDE_SKILL="$EVID/home/.claude/skills/plexi-cli/SKILL.md"
CODEX_SKILL="$EVID/home/.codex/skills/plexi-cli/SKILL.md"
if [[ -s "$CLAUDE_SKILL" && -s "$CODEX_SKILL" ]]; then
  record PASS step1 "installed under HOME/.claude and HOME/.codex"
else
  record FAIL step1 "$(cat "$EVID/logs/install.out" "$EVID/logs/install.err")"
  exit 1
fi

# ── 2. plexi_version matches this binary ─────────────────────────────────────
WANT="$("$BIN" --version | awk '{print $2}')"
GOT="$(python3 - "$CLAUDE_SKILL" <<'PY'
import re, sys
text=open(sys.argv[1]).read().split("---", 2)[1]
match=re.search(r'^plexi_version:\s*"([^"]+)"', text, re.M)
print(match.group(1) if match else "")
PY
)"
if [[ -n "$WANT" && "$GOT" == "$WANT" ]]; then
  record PASS step2 "plexi_version $GOT"
else
  record FAIL step2 "skill=$GOT binary=$WANT"
fi

# ── 3. Every plexi command named in the skill exists ─────────────────────────
if python3 - "$CLAUDE_SKILL" "$BIN" <<'PY'
import re, subprocess, sys
text = open(sys.argv[1]).read()
binary = sys.argv[2]
paths = set()
for chunk in re.findall(r"`([^`]+)`", text):
    if not chunk.startswith("plexi ") and not chunk.startswith("plexi\n"):
        continue
    tokens = []
    for tok in chunk.split():
        if tok == "plexi":
            continue
        if not re.fullmatch(r"[a-z0-9-]+", tok):
            break
        tokens.append(tok)
    if tokens:
        paths.add(tuple(tokens))
missing = []
for tokens in sorted(paths):
    run = subprocess.run([binary, *tokens, "--help"], capture_output=True, text=True)
    if run.returncode != 0:
        missing.append(" ".join(tokens))
if missing:
    print("missing: " + ", ".join(missing))
    raise SystemExit(1)
print(f"commands ok: {len(paths)}")
PY
then
  record PASS step3 "every named command exists"
else
  record FAIL step3 "command check failed"
fi

# ── 4. Skill lint from V1-03 step 10 ─────────────────────────────────────────
if grep -nE 'permission resolve|needs-you resolve|permissions allow|secret grant|secret exec' "$CLAUDE_SKILL" "$SKILL_SRC"; then
  record FAIL step4 "skill names a resolve command"
else
  record PASS step4 "skill lint clean"
fi

# ── 5. A scripted agent waits on permission_required ─────────────────────────
if ! pgrep -f "Xvfb $DISPLAY_NUM" >/dev/null 2>&1; then
  Xvfb "$DISPLAY_NUM" -screen 0 1400x900x24 >/tmp/xvfb-w7.log 2>&1 &
  XVFB_PID=$!
  sleep 0.4
fi
rm -rf "$WS"
mkdir -p "$WS"
( cd "$WS" && "$BIN" workspace init >"$EVID/logs/workspace-init.out" 2>"$EVID/logs/workspace-init.err" || true )
"$BIN" host stop >/dev/null 2>&1 || true
set +e
"$BIN" host start --ephemeral --timeout-secs 90 --pane "cwd=$WS" >"$EVID/logs/host-start.out" 2>"$EVID/logs/host-start.err"
HOST_RC=$?
set -e
"$BIN" host status --json >"$EVID/logs/host-status.json" 2>/dev/null || true
if [[ "$HOST_RC" -ne 0 ]] || ! grep -q '"ready"[[:space:]]*:[[:space:]]*true' "$EVID/logs/host-status.json"; then
  record FAIL step5 "host not ready"
  exit 1
fi
"$BIN" context set-root "$WS" >/dev/null 2>&1 || true
"$BIN" app install "$REPO/apps/chess" --yes >"$EVID/logs/app-install.out" 2>"$EVID/logs/app-install.err" || {
  record FAIL step5 "chess install failed"
  exit 1
}
"$BIN" app open chess --right >"$EVID/logs/open-chess.out" 2>"$EVID/logs/open-chess.err" || true

TERM=""
for _ in $(seq 1 30); do
  "$BIN" pane list >"$EVID/logs/panes.json" 2>/dev/null || true
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
  [[ -n "$TERM" ]] && break
  sleep 0.5
done
[[ -n "$TERM" ]] || { record FAIL step5 "no terminal"; exit 1; }

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
    "operation_id": "w7-e7e5-$$",
    "move": "e7e5",
}))
PY
)
P=""
for attempt in $(seq 1 12); do
  in_pane "ask-$attempt" "\"$BIN\" app call chess chess.play --json --input '$PLAY' > '$EVID/logs/ask.json' 2> '$EVID/logs/ask.err'" || true
  if grep -q 'tool_not_found' "$EVID/logs/ask.json" "$EVID/logs/ask.err" 2>/dev/null; then
    sleep 2
    continue
  fi
  P="$(python3 - "$EVID/logs/ask.json" "$EVID/logs/ask.err" <<'PY'
import re, sys
text=""
for path in sys.argv[1:]:
    try:
        text += open(path).read() + "\n"
    except OSError:
        pass
if "permission_required" not in text:
    print(""); raise SystemExit
match=re.search(r'"pending_request_id"\s*:\s*"([^"]+)"', text)
print(match.group(1) if match else "")
PY
)"
  [[ -n "$P" ]] && break
  sleep 2
done
[[ -n "$P" ]] || { record FAIL step5 "$(cat "$EVID/logs/ask.json" "$EVID/logs/ask.err" 2>/dev/null)"; exit 1; }
note "pending $P"
"$BIN" assistant permission list >"$EVID/logs/audit.json" 2>"$EVID/logs/audit.err" || true
if python3 - "$EVID/logs/audit.json" "$P" <<'PY'
import json, sys
data=json.load(open(sys.argv[1]))
pending=sys.argv[2]
refuses=[
    row for row in data.get("audit") or []
    if row.get("kind")=="refuse" and row.get("call_id")==pending
]
raise SystemExit(0 if not refuses else 1)
PY
then
  record PASS step5 "permission_required $P; agent did not resolve"
else
  record FAIL step5 "audit shows a self-resolve for $P"
fi

if grep -q $'FAIL\t' "$EVID/results.tsv"; then
  note "FAIL: V1-14"
  exit 1
fi
note "PASS: V1-14 steps 1-5"
exit 0

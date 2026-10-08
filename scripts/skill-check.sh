#!/usr/bin/env bash
# V1-14 installed skill check (W7).
#
# Installs the skill embedded in the binary, checks that it matches that
# binary, and runs one chess call the way the skill tells an agent to:
# print the pending id and stop.
#
# Linux writes the sealed permission audit only when Secret Service is up.
# Start a private session bus, the same way the other installed gate scripts do,
# so step 5 can ask chess and read the audit.
if [[ "$(uname -s)" == "Linux" && -z "${SKILL_CHECK_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env SKILL_CHECK_INNER=1 "$0" "$@"
fi
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
if [[ "$(uname -s)" == "Linux" ]] && command -v gnome-keyring-daemon >/dev/null 2>&1; then
  if ! gdbus introspect --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets >/dev/null 2>&1; then
    # A private session bus has no Secret Service until the daemon starts.
    # The host will not seal the permission audit without it.
    eval "$(gnome-keyring-daemon --start --components=secrets)"
    export GNOME_KEYRING_CONTROL SSH_AUTH_SOCK
  fi
fi

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
# Fenced blocks are stripped before inline spans are paired, because a ```
# fence leaves leftover backticks that would swallow the rest of the file.
# A token is a subcommand only when the parent's --help lists it. Anything
# after that is a positional and is not part of the command path.
if python3 - "$CLAUDE_SKILL" "$BIN" <<'PY'
import os, re, subprocess, sys
text = open(sys.argv[1]).read()
binary = sys.argv[2]

def is_command_word(tok):
    return bool(re.fullmatch(r"[a-z][a-z0-9-]*", tok))

def subcommands(prefix):
    run = subprocess.run(
        [binary, *prefix, "--help"],
        capture_output=True,
        text=True,
        env={**os.environ, "NO_COLOR": "1"},
    )
    if run.returncode != 0:
        return None
    # Root help groups commands under Workspace/Apps/… headings. Nested help
    # uses clap's Commands: list. Both print a name in the first column,
    # followed by at least two spaces and a description. Strip ANSI in case
    # the grouped help still paints the name.
    names = []
    for line in run.stdout.splitlines():
        line = re.sub(r"\x1b\[[0-9;]*m", "", line)
        match = re.match(r"  ([a-z][a-z0-9-]*)\s{2,}\S", line)
        if match and match.group(1) != "help":
            names.append(match.group(1))
    return names

def resolve(tokens):
    path = []
    saw_word = False
    for tok in tokens:
        if tok == "plexi":
            continue
        if not is_command_word(tok):
            break
        saw_word = True
        known = subcommands(path)
        if known is None or tok not in known:
            if not path:
                return None
            break
        path.append(tok)
    if not saw_word:
        return ()
    return tuple(path)

def strip_fences(src):
    buf, fences, cur = [], [], None
    for line in src.splitlines(True):
        stripped = line.lstrip()
        if stripped.startswith("```"):
            if cur is None:
                cur = [stripped[3:].strip(), []]
            else:
                fences.append((cur[0], "".join(cur[1])))
                cur = None
        elif cur is not None:
            cur[1].append(line)
        else:
            buf.append(line)
    return "".join(buf), fences

inline, fences = strip_fences(text)
named = []

def take(label, tokens):
    path = resolve(tokens)
    if path is None:
        named.append((label, None))
    elif path:
        named.append((label, path))

for chunk in re.findall(r"`([^`\n]+)`", inline):
    idx = chunk.find("plexi")
    if idx < 0:
        continue
    tokens = chunk[idx:].split()
    if tokens and tokens[0] == "plexi":
        take(chunk.strip(), tokens)

for lang, body in fences:
    lines = body.splitlines()
    if lang == "":
        for line in lines:
            if not line.strip() or line[:1].isspace():
                continue
            take(line.strip(), line.split())
    elif lang in ("bash", "sh"):
        for line in lines:
            normalized = "".join(" " if c in "()|;&`$" else c for c in line)
            tokens = normalized.split()
            i = 0
            while i < len(tokens):
                if tokens[i] != "plexi":
                    i += 1
                    continue
                j = i + 1
                while j < len(tokens) and tokens[j] != "plexi":
                    j += 1
                take(" ".join(tokens[i:j]), tokens[i:j])
                i = j

missing = []
seen = set()
for label, path in named:
    if path is None:
        missing.append(label)
        continue
    seen.add(path)
if len(seen) < 20:
    print(f"extraction collapse: only {len(seen)} commands")
    raise SystemExit(1)
if missing:
    print("missing: " + ", ".join(missing))
    raise SystemExit(1)
print(f"commands ok: {len(seen)}")
PY
then
  record PASS step3 "every named command exists"
else
  record FAIL step3 "command check failed"
fi

# ── 4. Skill lint from V1-03 step 10 ─────────────────────────────────────────
# Alpha names resolve, allow, grant, and exec only to say they are refused.
# A mention with no refusal in the same sentence still fails.
if python3 - "$CLAUDE_SKILL" "$SKILL_SRC" <<'PY'
import re, sys
pat = re.compile(r"permission resolve|needs-you resolve|permissions allow|secret grant|secret exec")
refusal = re.compile(
    r"refus|do not grant|does not grant|permission_denied|agents do not grant|do not approve",
    re.I,
)
bad = []
for path in sys.argv[1:]:
    lines = open(path).read().splitlines()
    for i, line in enumerate(lines):
        if not pat.search(line):
            continue
        window = " ".join(lines[i:i + 3])
        if not refusal.search(window):
            bad.append(f"{path}:{i + 1}:{line}")
if bad:
    print("skill names a resolve command without a refusal:")
    print("\n".join(bad))
    raise SystemExit(1)
PY
then
  record PASS step4 "skill lint clean"
else
  record FAIL step4 "skill names a resolve command"
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

#!/usr/bin/env bash
# V1 acceptance suite — one line per V1-CONTRACT item.
#
# Runs against an installed build. Feature scripts already in this tree are
# invoked by path. A script or command that is not on the build is NOT-LANDED.
# An approval that did not go through scripts/e2e/human.sh (HUMAN_APPROVE)
# is VERIFIED-VIA-BYPASS, never PASS.
#
#   scripts/v1-acceptance.sh
#   scripts/v1-acceptance.sh --bin ~/.local/bin/plexi-alpha
#   scripts/v1-acceptance.sh --pr 2704
#
# Linux: private Xvfb, a temp HOME, PLEXI_KEYCHAIN_PATH pointed at a throwaway
# file, a local Docker relay when the phone script is present, and the mock
# OpenRouter unless OPENROUTER_API_KEY is already set.
#
# Exit 0 when every item is PASS, 1 when any item is FAIL, 2 when the run is
# incomplete (NOT-LANDED or VERIFIED-VIA-BYPASS and no FAIL).

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EVID="${V1_ACCEPTANCE_EVID:-/tmp/plexi-v1-acceptance-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$EVID"

PR_NUM=""
BIN_ARG=""
ONLY="${V1_ACCEPTANCE_ONLY:-}"

usage() {
  cat <<'EOF'
usage: scripts/v1-acceptance.sh [--bin PATH] [--pr N]

  --bin PATH   installed channel binary (plexi-alpha, plexi-pr-N, …)
  --pr N       use plexi-pr-N from PATH (after just pr-install N)

Env:
  PLEXI_BIN              same as --bin
  OPENROUTER_API_KEY     when set, the suite does not start the mock
  V1_ACCEPTANCE_ONLY     comma-separated item ids (V1-01,V1-14) for a partial run
  V1_ACCEPTANCE_EVID     directory for per-item logs (kept after the run)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin)
      BIN_ARG="${2:-}"
      shift 2
      ;;
    --pr)
      PR_NUM="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

# Caller pane vars would address the wrong host.
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_CALL_CREDENTIAL || true

REAL_HOME="${HOME}"
WORK_HOME="$(mktemp -d "${TMPDIR:-/tmp}/plexi-v1-home.XXXXXX")"
export HOME="$WORK_HOME"
mkdir -p "$HOME"
# Never the login keychain. Channel scripts that need a keychain file inherit
# this; ones that create their own throwaway override it.
export PLEXI_KEYCHAIN_PATH="$HOME/v1-acceptance.keychain"
export PLEXI_KEYCHAIN_PASSWORD
PLEXI_KEYCHAIN_PASSWORD="$(openssl rand -hex 24 2>/dev/null || python3 -c 'import secrets; print(secrets.token_hex(24))')"
: >"$PLEXI_KEYCHAIN_PATH"
# Do not attach to a user secret service. Scripts that need a private bus
# start one with dbus-run-session.
unset DBUS_SESSION_BUS_ADDRESS || true

XVFB_PID=""
MOCK_PID=""
DISPLAY_NUM=""

cleanup() {
  if [[ -n "${BIN:-}" && -x "${BIN:-}" ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$MOCK_PID" ]]; then
    kill "$MOCK_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$XVFB_PID" ]]; then
    kill "$XVFB_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "${WORK_HOME:-}" && "$WORK_HOME" == *plexi-v1-home* ]]; then
    rm -rf "$WORK_HOME"
  fi
}
trap cleanup EXIT

start_xvfb() {
  local n
  for n in $(seq 120 160); do
    if [[ ! -e "/tmp/.X${n}-lock" ]]; then
      DISPLAY_NUM=":$n"
      break
    fi
  done
  if [[ -z "$DISPLAY_NUM" ]]; then
    echo "FAIL setup: no free X display" >&2
    exit 1
  fi
  Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -ac +extension GLX +render -noreset \
    >"$EVID/xvfb.log" 2>&1 &
  XVFB_PID=$!
  export DISPLAY="$DISPLAY_NUM"
  local i
  for i in $(seq 1 50); do
    if command -v xdpyinfo >/dev/null 2>&1 && xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -S "/tmp/.X11-unix/X${DISPLAY_NUM#:}" ]]; then
      return 0
    fi
    sleep 0.1
  done
  echo "FAIL setup: Xvfb $DISPLAY_NUM did not come up" >&2
  cat "$EVID/xvfb.log" >&2 || true
  exit 1
}

if ! command -v Xvfb >/dev/null 2>&1; then
  echo "FAIL setup: Xvfb is not installed" >&2
  exit 1
fi
start_xvfb

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-ubuntu}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true
if [[ -f /usr/share/vulkan/icd.d/lvp_icd.json ]]; then
  export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
  export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
fi
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"

resolve_bin() {
  local candidate="" base
  if [[ -n "$BIN_ARG" ]]; then
    candidate="$BIN_ARG"
  elif [[ -n "$PR_NUM" ]]; then
    candidate="$(command -v "plexi-pr-${PR_NUM}" 2>/dev/null || true)"
    if [[ -z "$candidate" && -x "$REAL_HOME/.local/bin/plexi-pr-${PR_NUM}" ]]; then
      candidate="$REAL_HOME/.local/bin/plexi-pr-${PR_NUM}"
    fi
  elif [[ -n "${PLEXI_BIN:-}" ]]; then
    candidate="$PLEXI_BIN"
  elif command -v plexi-alpha >/dev/null 2>&1; then
    candidate="$(command -v plexi-alpha)"
  elif [[ -x "$REPO/target/release/plexi" ]]; then
    candidate="$REPO/target/release/plexi"
  elif command -v plexi >/dev/null 2>&1; then
    candidate="$(command -v plexi)"
  fi
  if [[ -z "$candidate" || ! -x "$candidate" ]]; then
    echo ""
    return
  fi
  if [[ "$candidate" != /* ]]; then
    candidate="$(command -v "$candidate")"
  fi
  # current_exe() resolves symlinks, so a symlink named plexi-alpha still
  # reports the bare binary and loses the channel. Hand the suite a real
  # file whose basename is the channel name when the caller passed one.
  base="$(basename "$candidate")"
  if [[ -L "$candidate" && "$base" == plexi-* ]]; then
    local copy="$EVID/bin/$base"
    mkdir -p "$EVID/bin"
    cp -f "$(readlink -f "$candidate")" "$copy"
    chmod +x "$copy"
    candidate="$copy"
  fi
  printf '%s' "$candidate"
}

BIN="$(resolve_bin)"
BIN_BASE=""
if [[ -n "$BIN" ]]; then
  BIN_BASE="$(basename "$BIN")"
  case "$BIN_BASE" in
    plexi-pr-*)
      PR_NUM="${PR_NUM:-${BIN_BASE#plexi-pr-}}"
      unset PLEXI_CHANNEL || true
      ;;
    plexi-*)
      unset PLEXI_CHANNEL || true
      ;;
    plexi)
      if [[ -z "${PLEXI_CHANNEL:-}" ]]; then
        export PLEXI_CHANNEL="alpha"
      fi
      ;;
  esac
  export PATH="$(dirname "$BIN"):$PATH"
fi

# Mock OpenRouter unless the caller already supplied a real key.
OPENROUTER_MODE="mock"
if [[ -n "${OPENROUTER_API_KEY:-}" ]]; then
  OPENROUTER_MODE="real key"
  unset PLEXI_OPENROUTER_BASE_URL || true
elif [[ -f "$REPO/scripts/e2e/ledger/mock_openrouter.py" ]]; then
  python3 "$REPO/scripts/e2e/ledger/mock_openrouter.py" >"$EVID/mock-openrouter.out" 2>&1 &
  MOCK_PID=$!
  mock_port=""
  for _ in $(seq 1 50); do
    mock_port="$(sed -n 's/^PORT=//p' "$EVID/mock-openrouter.out" | head -1)"
    if [[ -n "$mock_port" ]]; then
      break
    fi
    if ! kill -0 "$MOCK_PID" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
  if [[ -n "$mock_port" ]]; then
    export PLEXI_OPENROUTER_BASE_URL="http://127.0.0.1:${mock_port}/v1"
    export OPENROUTER_API_KEY="sk-v1-acceptance-mock"
  else
    OPENROUTER_MODE="mock unavailable"
  fi
else
  OPENROUTER_MODE="mock script not on this tree"
  unset OPENROUTER_API_KEY || true
fi

# Phone tests talk to the relay container the feature script starts locally.
# Drop any ambient staging URL so a landed script cannot leave the machine.
unset PLEXI_RELAY_URL RELAY_URL RELAY_BASE_URL || true

declare -a ROWS=()
N_PASS=0
N_FAIL=0
N_LANDED=0
N_BYPASS=0

note_row() {
  local id="$1" status="$2" detail="$3"
  ROWS+=("$id"$'\t'"$status"$'\t'"$detail")
  printf '%s\t%s\t%s\n' "$id" "$status" "$detail"
  case "$status" in
    PASS) N_PASS=$((N_PASS + 1)) ;;
    FAIL) N_FAIL=$((N_FAIL + 1)) ;;
    NOT-LANDED) N_LANDED=$((N_LANDED + 1)) ;;
    VERIFIED-VIA-BYPASS) N_BYPASS=$((N_BYPASS + 1)) ;;
  esac
}

skip_item() {
  [[ -z "$ONLY" ]] && return 1
  local id="$1"
  case ",$ONLY," in
    *",$id,"*) return 1 ;;
    *) return 0 ;;
  esac
}

# Probe: exit 0 when the binary accepts the subcommand path.
binary_has() {
  [[ -n "$BIN" && -x "$BIN" ]] || return 1
  local out
  out="$("$BIN" "$@" --help 2>&1)" || true
  if grep -qiE 'unrecognized subcommand|unrecognized command' <<<"$out"; then
    return 1
  fi
  "$BIN" "$@" --help >/dev/null 2>&1
}

# human | bypass | none | driver-missing
script_approval_mode() {
  local script="$1"
  python3 - "$script" "$REPO/scripts/e2e/human.sh" <<'PY'
import re, sys
path, human = sys.argv[1:]
text = open(path, encoding="utf-8", errors="replace").read()
human_ok = open(human, encoding="utf-8", errors="replace").read().find("HUMAN_APPROVE") >= 0 if __import__("os").path.isfile(human) else False
wants_human = "HUMAN_APPROVE" in text and re.search(r"human\.sh", text)
positive = []
pat = re.compile(
    r"permission resolve|needs-you resolve|permissions allow|secret grant|secret exec|"
    r"command-view resolve|\bchanges accept\b|\bresolve\b[^\n]*--approve"
)
for raw in text.splitlines():
    line = raw.split("#", 1)[0]
    if not pat.search(line):
        continue
    # Attack attempts and the skill-lint grep are not the approving path.
    if re.search(r"in_pane|\|\| true|permission_denied|\bgrep\b", line):
        continue
    if re.search(r"^\s*(pass|fail|ok|bad|record)\b", line):
        continue
    positive.append(line.strip())
if wants_human and not human_ok:
    print("driver-missing")
elif wants_human and not positive:
    print("human")
elif positive or "VERIFIED-VIA-BYPASS" in text:
    print("bypass")
else:
    print("none")
PY
}

script_needs_pr() {
  grep -qE '\$\{1:\?[^}]*<PR>' "$1"
}

invoke_script() {
  local script="$1"
  local log="$2"
  local mode
  mode="$(script_approval_mode "$script")"
  if [[ "$mode" == "driver-missing" ]]; then
    printf 'driver-missing\n' >"$log"
    echo "driver-missing"
    return 0
  fi
  export PLEXI_BIN="$BIN"
  export BIN
  local -a cmd
  if script_needs_pr "$script"; then
    local pr="$PR_NUM"
    if [[ -z "$pr" && "$BIN_BASE" == plexi-pr-* ]]; then
      pr="${BIN_BASE#plexi-pr-}"
    fi
    if [[ -z "$pr" ]]; then
      printf 'pr-only\n' >"$log"
      echo "pr-only"
      return 0
    fi
    # The script looks up plexi-pr-<N> on PATH. current_exe() follows
    # symlinks, so this has to be a hardlink (or a copy) whose path basename
    # is the channel name.
    mkdir -p "$EVID/bin"
    if [[ "$BIN_BASE" != "plexi-pr-${pr}" ]]; then
      ln -f "$BIN" "$EVID/bin/plexi-pr-${pr}" 2>/dev/null || cp -f "$BIN" "$EVID/bin/plexi-pr-${pr}"
      chmod +x "$EVID/bin/plexi-pr-${pr}" || true
    fi
    export PATH="$EVID/bin:$(dirname "$BIN"):$PATH"
    cmd=(timeout --foreground 1200 bash "$script" "$pr")
  elif grep -q 'path-to-plexi-binary' "$script"; then
    cmd=(timeout --foreground 1200 bash "$script" "$BIN")
  else
    cmd=(timeout --foreground 1200 bash "$script")
  fi
  set +e
  "${cmd[@]}" >"$log" 2>&1
  local code=$?
  set +e
  if [[ "$code" -eq 0 ]]; then
    echo "$mode"
  else
    echo "fail:$code"
  fi
}

# Run one or more scripts. A required path that is missing → NOT-LANDED.
# Paths after --any are alternates (a later PR renamed the script): the item
# is landed when any one of them exists, and every one that exists is run.
# Paths after --optional run when present and are never required.
# requires_human=1: a green run that never called HUMAN_APPROVE is bypass.
run_scripts() {
  local id="$1" title="$2" requires_human="$3"
  shift 3
  local -a required=() alternates=() optional=()
  local section="required"
  local arg
  for arg in "$@"; do
    if [[ "$arg" == "--any" ]]; then
      section="any"
      continue
    fi
    if [[ "$arg" == "--optional" ]]; then
      section="optional"
      continue
    fi
    if [[ "$section" == "required" ]]; then
      required+=("$arg")
    elif [[ "$section" == "any" ]]; then
      alternates+=("$arg")
    else
      optional+=("$arg")
    fi
  done

  local path missing="" any_hit=0
  for path in "${required[@]}"; do
    if [[ ! -f "$REPO/$path" ]]; then
      missing+=" $path"
    fi
  done
  if [[ ${#alternates[@]} -gt 0 ]]; then
    for path in "${alternates[@]}"; do
      if [[ -f "$REPO/$path" ]]; then
        any_hit=1
      fi
    done
    if [[ "$any_hit" -eq 0 ]]; then
      for path in "${alternates[@]}"; do
        missing+=" $path"
      done
    fi
  fi
  if [[ -n "$missing" ]]; then
    note_row "$id" "NOT-LANDED" "missing:${missing# }"
    return
  fi

  if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    note_row "$id" "FAIL" "no installed binary"
    return
  fi

  local -a present=()
  for path in "${required[@]}"; do
    present+=("$path")
  done
  for path in "${alternates[@]}"; do
    if [[ -f "$REPO/$path" ]]; then
      present+=("$path")
    fi
  done
  for path in "${optional[@]}"; do
    if [[ -f "$REPO/$path" ]]; then
      present+=("$path")
    fi
  done

  if [[ ${#present[@]} -eq 0 ]]; then
    note_row "$id" "FAIL" "no acceptance script ran"
    return
  fi

  local saw_bypass=0 saw_human=0 saw_none=0
  local detail="" path mode log
  for path in "${present[@]}"; do
    log="$EVID/${id}-$(basename "$path").log"
    mode="$(invoke_script "$REPO/$path" "$log")"
    case "$mode" in
      driver-missing)
        note_row "$id" "NOT-LANDED" "$path requires scripts/e2e/human.sh (W15), which is not on this tree"
        return
        ;;
      pr-only)
        note_row "$id" "NOT-LANDED" "$path addresses plexi-pr-<N> only; this install is $BIN_BASE"
        return
        ;;
      fail:*)
        note_row "$id" "FAIL" "$path exited ${mode#fail:} (log $log)"
        return
        ;;
      human) saw_human=1; detail+=" $path" ;;
      bypass) saw_bypass=1; detail+=" $path" ;;
      none) saw_none=1; detail+=" $path" ;;
      *)
        note_row "$id" "FAIL" "$path returned unexpected status $mode"
        return
        ;;
    esac
  done

  if [[ "$saw_bypass" -eq 1 || ( "$requires_human" -eq 1 && "$saw_human" -eq 0 ) ]]; then
    note_row "$id" "VERIFIED-VIA-BYPASS" "ran${detail}; approvals did not use HUMAN_APPROVE"
    return
  fi
  note_row "$id" "PASS" "ran${detail}"
}

item_v1_01() {
  local id="V1-01"
  if skip_item "$id"; then
    return
  fi
  if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    note_row "$id" "FAIL" "no installed binary (just pr-install <N> or install plexi-alpha)"
    return
  fi
  local log="$EVID/V1-01.log"
  : >"$log"
  "$BIN" host stop >>"$log" 2>&1 || true
  local out code
  set +e
  out="$("$BIN" host start --ephemeral --timeout-secs 90 2>&1)"
  code=$?
  set +e
  printf '%s\n' "$out" >>"$log"
  if [[ "$code" -ne 0 ]]; then
    note_row "$id" "FAIL" "host start exited $code (log $log)"
    return
  fi
  set +e
  out="$("$BIN" host status --json 2>&1)"
  code=$?
  set +e
  printf '%s\n' "$out" >>"$log"
  if [[ "$code" -ne 0 ]] || ! grep -q '"ready"[[:space:]]*:[[:space:]]*true' <<<"$out"; then
    note_row "$id" "FAIL" "host status did not report ready:true"
    "$BIN" host stop >>"$log" 2>&1 || true
    return
  fi
  "$BIN" host stop >>"$log" 2>&1 || true

  local ws="$EVID/v1-01-ws"
  mkdir -p "$ws"
  set +e
  out="$(cd "$ws" && "$BIN" workspace init 2>&1)"
  code=$?
  set +e
  printf '%s\n' "$out" >>"$log"
  if [[ "$code" -ne 0 ]]; then
    note_row "$id" "FAIL" "workspace init exited $code"
    return
  fi
  local channel_dir
  channel_dir="$(printf '%s\n' "$out" | sed -n 's/^  channel dir:[[:space:]]*//p' | head -1 | tr -d '/')"
  if [[ -z "$channel_dir" || ! -d "$ws/$channel_dir" ]]; then
    note_row "$id" "FAIL" "workspace init did not name a channel dir"
    return
  fi
  if [[ "$channel_dir" == ".plexi" ]]; then
    note_row "$id" "FAIL" "binary $BIN_BASE has no legacy channel agents dir; use plexi-alpha or plexi-pr-N"
    return
  fi
  local legacy="$ws/$channel_dir/agents/writer"
  mkdir -p "$legacy"
  printf 'legacy-writer-marker\n' >"$legacy/AGENT.md"
  set +e
  out="$(cd "$ws" && "$BIN" agent list 2>&1)"
  code=$?
  set +e
  printf '%s\n' "$out" >>"$log"
  if [[ "$code" -ne 0 ]] || ! grep -q 'writer' <<<"$out"; then
    note_row "$id" "FAIL" "agent list did not show the migrated definition"
    return
  fi
  local canonical="$ws/.plexi/agents/writer/AGENT.md"
  if [[ ! -f "$canonical" ]] || ! grep -q 'legacy-writer-marker' "$canonical"; then
    note_row "$id" "FAIL" "definition was not copied to .plexi/agents/"
    return
  fi
  if [[ -d "$legacy" ]]; then
    note_row "$id" "FAIL" "legacy agents dir was still present after list"
    return
  fi
  printf 'canonical-writer-marker\n' >"$canonical"
  mkdir -p "$legacy"
  printf 'stale-legacy-marker\n' >"$legacy/AGENT.md"
  set +e
  out="$(cd "$ws" && "$BIN" agent list 2>&1)"
  code=$?
  set +e
  printf '%s\n' "$out" >>"$log"
  if [[ "$code" -ne 0 ]]; then
    note_row "$id" "FAIL" "second agent list exited $code"
    return
  fi
  if ! grep -q 'canonical-writer-marker' "$canonical" || grep -q 'stale-legacy-marker' "$canonical"; then
    note_row "$id" "FAIL" "legacy agents dir was re-read over .plexi/agents/"
    return
  fi
  note_row "$id" "PASS" "host ready; .plexi/agents holds the definition; legacy dir not re-read"
}

item_v1_14() {
  local id="V1-14"
  if skip_item "$id"; then
    return
  fi
  local skill="$REPO/skills/plexi-cli/SKILL.md"
  if [[ -f "$REPO/scripts/skill-check.sh" ]]; then
    local check_log="$EVID/V1-14-skill-check.log"
    local check_mode
    check_mode="$(invoke_script "$REPO/scripts/skill-check.sh" "$check_log")"
    case "$check_mode" in
      fail:*)
        note_row "$id" "FAIL" "scripts/skill-check.sh exited ${check_mode#fail:} (log $check_log)"
        return
        ;;
      driver-missing|pr-only)
        note_row "$id" "NOT-LANDED" "scripts/skill-check.sh could not run ($check_mode)"
        return
        ;;
      bypass)
        note_row "$id" "VERIFIED-VIA-BYPASS" "scripts/skill-check.sh did not use HUMAN_APPROVE"
        return
        ;;
    esac
  fi
  if [[ ! -f "$skill" ]]; then
    note_row "$id" "NOT-LANDED" "skills/plexi-cli/SKILL.md is not on this tree"
    return
  fi
  if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    note_row "$id" "FAIL" "no installed binary to compare with the skill"
    return
  fi
  local log="$EVID/V1-14.log"
  python3 - "$skill" "$BIN" >"$log" 2>&1 <<'PY'
import re, subprocess, sys
skill_path, binary = sys.argv[1:]
text = open(skill_path, encoding="utf-8").read()
errors = []

fm = re.match(r"^---\n(.*?)\n---", text, re.S)
plexi_version = ""
if fm:
    m = re.search(r'^plexi_version:\s*"?([^"\n]+)"?', fm.group(1), re.M)
    if m:
        plexi_version = m.group(1).strip()
if not plexi_version:
    errors.append("skill frontmatter has no plexi_version")

ver = subprocess.run([binary, "--version"], capture_output=True, text=True)
ver_out = (ver.stdout or ver.stderr or "").strip()
bin_version = ""
m = re.search(r"(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)", ver_out)
if m:
    bin_version = m.group(1)
else:
    errors.append(f"could not parse binary version from {ver_out!r}")
if plexi_version and bin_version and plexi_version != bin_version:
    errors.append(f"plexi_version {plexi_version} != binary {bin_version}")

lint = re.compile(
    r"permission resolve|needs-you resolve|permissions allow|secret grant|secret exec"
)
hits = [f"{i}:{line.strip()}" for i, line in enumerate(text.splitlines(), 1) if lint.search(line)]
if hits:
    errors.append("skill tells agents to self-resolve: " + "; ".join(hits[:6]))
else:
    wait = re.compile(r"permission_required|never resolve|do not resolve|\bwait\b", re.I)
    if not wait.search(text):
        errors.append("skill does not tell agents to wait on permission_required")

def is_command_word(tok):
    return bool(tok) and tok[0].islower() and all(c.islower() or c.isdigit() or c == "-" for c in tok)

help_cache = {}

def help_text(path):
    key = tuple(path)
    if key not in help_cache:
        proc = subprocess.run([binary, *path, "--help"], capture_output=True, text=True)
        help_cache[key] = (proc.returncode, (proc.stdout or "") + "\n" + (proc.stderr or ""))
    return help_cache[key]

def subcommands(path):
    code, body = help_text(path)
    if code != 0 and "unrecognized" in body.lower():
        return set()
    # Root help groups verbs under headings (Workspace, Panes, …) instead of
    # a Commands: block. Nested help uses Commands:. Both indent the name,
    # then at least two spaces, then the description.
    names = set()
    for line in body.splitlines():
        m = re.match(r"^  ([a-z][a-z0-9-]*)\s{2,}\S", line)
        if m:
            names.add(m.group(1))
    return names

def walk(tokens, strict):
    path = []
    for tok in tokens:
        if not is_command_word(tok):
            break
        subs = subcommands(path)
        if tok in subs:
            path.append(tok)
            continue
        if strict:
            return path, tok
        break
    return path, None

checked = set()
missing = []

def check(tokens, strict, where):
    if not tokens:
        return
    path, bad = walk(tokens, strict)
    if bad is not None:
        missing.append(f"{where}: plexi {' '.join(path + [bad])}")
        return
    if not path:
        if strict:
            missing.append(f"{where}: no subcommand in {' '.join(tokens)}")
        return
    key = tuple(path)
    if key in checked:
        return
    checked.add(key)
    code, body = help_text(path)
    if code != 0 or "unrecognized subcommand" in body.lower():
        missing.append(f"{where}: plexi {' '.join(path)} --help failed")

# Bare fences are the reference list: every leading command word must exist.
# bash fences and inline `plexi …` names are walked until a positional.
fence = False
lang = ""
buf = []
blocks = []
for line in text.splitlines():
    stripped = line.lstrip()
    if stripped.startswith("```"):
        if fence:
            blocks.append((lang, buf))
            fence = False
            buf = []
        else:
            fence = True
            lang = stripped[3:].strip()
        continue
    if fence:
        buf.append(line)

for lang, lines in blocks:
    if lang in ("json", "toml", "rust"):
        continue
    if lang in ("", "text"):
        for line in lines:
            if not line.strip() or line[:1].isspace():
                continue
            check(line.split(), True, "reference")
    elif lang in ("bash", "sh", "shell"):
        for line in lines:
            if "plexi" not in line:
                continue
            toks = line.replace("$(", " ").replace("`", " ").split()
            if "plexi" not in toks:
                continue
            rest = toks[toks.index("plexi") + 1:]
            check(rest, False, "example")

for m in re.finditer(r"`plexi ([^`]+)`", text):
    check(m.group(1).split(), False, "prose")

if missing:
    errors.append("named commands missing from the binary: " + "; ".join(missing[:12]))
if not checked:
    errors.append("skill named no plexi commands")

print(f"plexi_version={plexi_version} binary={bin_version} commands={len(checked)}")
if errors:
    for err in errors:
        print("FAIL " + err)
    raise SystemExit(1)
print("PASS skill version matches, named commands exist, skill does not teach self-resolve")
PY
  local code=$?
  if [[ "$code" -eq 0 ]]; then
    local summary
    summary="$(tail -1 "$log")"
    note_row "$id" "PASS" "$summary"
  else
    local why
    why="$(grep '^FAIL ' "$log" | head -3 | sed 's/^FAIL //' | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
    note_row "$id" "FAIL" "${why:-skill check failed (log $log)}"
  fi
}

run_one() {
  local id="$1" requires_human="$2"
  shift 2
  if skip_item "$id"; then
    return
  fi
  # Optional probe words follow a '|' in the last required path? No: probes
  # are passed as --probe a b before the paths.
  local -a probes=()
  local -a rest=()
  if [[ "${1:-}" == "--probe" ]]; then
    shift
    while [[ $# -gt 0 && "$1" != "--" && "$1" != "--optional" && "$1" != scripts/* && "$1" != services/* ]]; do
      probes+=("$1")
      shift
    done
  fi
  if [[ "${1:-}" == "--" ]]; then
    shift
  fi
  rest=("$@")
  local missing="" path section="required" any_hit=0
  local -a alternates=()
  for path in "${rest[@]}"; do
    if [[ "$path" == "--any" ]]; then
      section="any"
      continue
    fi
    if [[ "$path" == "--optional" ]]; then
      section="optional"
      continue
    fi
    if [[ "$section" == "required" && ! -f "$REPO/$path" ]]; then
      missing+=" $path"
    elif [[ "$section" == "any" ]]; then
      alternates+=("$path")
      if [[ -f "$REPO/$path" ]]; then
        any_hit=1
      fi
    fi
  done
  if [[ ${#alternates[@]} -gt 0 && "$any_hit" -eq 0 ]]; then
    for path in "${alternates[@]}"; do
      missing+=" $path"
    done
  fi
  if [[ -n "$missing" ]]; then
    note_row "$id" "NOT-LANDED" "missing:${missing# }"
    return
  fi
  if [[ ${#probes[@]} -gt 0 ]]; then
    if [[ -z "$BIN" || ! -x "$BIN" ]]; then
      note_row "$id" "FAIL" "no installed binary"
      return
    fi
    if ! binary_has "${probes[@]}"; then
      note_row "$id" "NOT-LANDED" "binary has no: ${probes[*]}"
      return
    fi
  fi
  if [[ "$id" == "V1-08" ]] && ! command -v docker >/dev/null 2>&1; then
    note_row "$id" "FAIL" "local Docker relay required and docker is not installed"
    return
  fi
  run_scripts "$id" "$id" "$requires_human" "${rest[@]}"
}

echo "v1-acceptance"
echo "binary    ${BIN:-<none>} (${BIN_BASE:-})"
if [[ -n "$BIN" ]]; then
  echo "version   $("$BIN" --version 2>&1 | head -1)"
fi
echo "tree      $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo "display   $DISPLAY"
echo "home      $HOME"
echo "keychain  $PLEXI_KEYCHAIN_PATH"
echo "openrouter $OPENROUTER_MODE"
echo "evidence  $EVID"
echo

item_v1_01
# Paths are the files those PRs actually add. --any lists a rename: either
# file lands the item, and every file that is present is run.
#   V1-02 #2720 scripts/permission-gate-e2e.sh (driver: #2709 scripts/e2e/human.sh)
#   V1-03 #2720 scripts/no-self-approval-e2e.sh
#   V1-04 #2718 scripts/permissions-seal-e2e.sh
#   V1-05 #2704 scripts/needs-you-e2e.sh + #2713 scripts/needs-you-persist-e2e.sh
#   V1-06 #2705/#2719 scripts/folder-secrets-e2e.sh (same path)
#   V1-07 #2710 scripts/e2e/ledger/run.sh
#   V1-08 #2708 services/relay/e2e_installed.sh
#   V1-09 #2706 scripts/e2e_agents_api_installed.sh
#   V1-10 #2715 scripts/multi-lead-e2e.sh + #2716 scripts/headless-queue-e2e.sh
#   V1-11 #2717 scripts/command-view-steer-e2e.sh (#2707 command-view-e2e.sh superseded)
#   V1-12 #2724 scripts/change-sets-e2e.sh (#2695 assistant-editor-change-set-e2e.sh superseded)
#   V1-13 #2703 scripts/cloud-basics-e2e.sh
#   V1-14 #2722 scripts/skill-check.sh
#   V1-15 #2718 scripts/app-share-e2e.sh
run_one V1-02 1 --probe assistant permission -- scripts/permission-gate-e2e.sh
run_one V1-03 1 -- scripts/no-self-approval-e2e.sh
run_one V1-04 1 --probe permissions -- scripts/permissions-seal-e2e.sh
run_one V1-05 1 --probe needs-you -- scripts/needs-you-e2e.sh scripts/needs-you-persist-e2e.sh
run_one V1-06 1 -- scripts/folder-secrets-e2e.sh
run_one V1-07 0 --probe ledger -- scripts/e2e/ledger/run.sh
run_one V1-08 1 -- services/relay/e2e_installed.sh
run_one V1-09 1 --probe agent head -- scripts/e2e_agents_api_installed.sh
run_one V1-10 0 --probe agent head -- scripts/multi-lead-e2e.sh scripts/headless-queue-e2e.sh
run_one V1-11 0 --probe command-view -- --any scripts/command-view-steer-e2e.sh scripts/command-view-e2e.sh
run_one V1-12 1 --probe changes -- --any scripts/change-sets-e2e.sh scripts/assistant-editor-change-set-e2e.sh
run_one V1-13 0 -- scripts/cloud-basics-e2e.sh
item_v1_14
run_one V1-15 1 --probe app package -- scripts/app-share-e2e.sh

echo

verdict="PASS"
exit_code=0
if [[ "$N_FAIL" -gt 0 ]]; then
  verdict="FAIL"
  exit_code=1
elif [[ "$N_LANDED" -gt 0 || "$N_BYPASS" -gt 0 ]]; then
  verdict="INCOMPLETE"
  exit_code=2
fi
echo "VERDICT ${verdict}  pass=${N_PASS} fail=${N_FAIL} not-landed=${N_LANDED} verified-via-bypass=${N_BYPASS}"
exit "$exit_code"

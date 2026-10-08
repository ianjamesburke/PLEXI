#!/usr/bin/env bash
# V1 acceptance suite — one line per V1-CONTRACT item.
#
# The acceptance target is the alpha channel binary built from this tree.
# `just build` produces target/release/plexi; the harness copies that file to
# a real binary named plexi-alpha and drives that. A channel-named binary
# ignores PLEXI_CHANNEL. The profile is ~/.plexi-alpha. This is not an
# integration branch and not a plexi-pr-N install. --bin and --pr are
# explicit overrides for a re-run, not the default.
#
# Feature scripts already in this tree are invoked by path. A script or
# command that is not on the build is NOT-LANDED. An approval that did not
# go through scripts/e2e/human.sh (HUMAN_APPROVE) is a FAIL. The row names
# the bypass: skip panes, CLI approval, VIA-BYPASS, env flag, or none when
# the contract requires a click and the script never makes one. A lint that
# names a refused command is not an approval. No contract item permits a
# bypass. See scripts/e2e/plexi-bin.md.
#
#   scripts/v1-acceptance.sh
#   scripts/v1-acceptance.sh --bin ~/.local/bin/plexi-alpha
#
# Item scripts take the channel binary from PLEXI_BIN. The harness exports
# that and PLEXI_E2E_SHIM (scripts/e2e/plexi-bin.sh). Scripts that still
# require plexi-pr-<N> are run from a same-directory copy that sources the
# shim. The owning PR applies the one-line in scripts/e2e/plexi-bin.md.
#
# Linux: private Xvfb, a temp HOME, PLEXI_KEYCHAIN_PATH pointed at a throwaway
# file, a local Docker relay when the phone script is present, and the mock
# OpenRouter unless OPENROUTER_API_KEY is already set. config.toml is
# snapshotted from scripts/default-config.toml and restored after every item
# so a gate or relay rewrite of [ai] backend cannot poison the ledger item.
# Between items the harness reaps Xvfb processes the item started. An
# xvfb-run left on :99 with an auth cookie makes xdpyinfo report that display
# down, and the next script then starts a host on a server it does not own.
#
# Before any item, a preflight installs or verifies VM dependencies:
# Python Pillow (V1-02 board clicks), Docker (V1-08 relay), tesseract
# (V1-11 reads the command pane), and an unlocked gnome-keyring on a private
# session bus (the Linux journal / sealed audit). A dependency that cannot
# be made to work is ENV-FAIL on the items that need it, not a product FAIL.
#
# Exit 0 when every item is PASS, 1 when any item is FAIL (including a
# bypass), 3 when the only failures are ENV-FAIL, 2 when the run is
# incomplete (NOT-LANDED and no FAIL or ENV-FAIL).

set -uo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HARNESS_DIR/.." && pwd)"
SHIM="$HARNESS_DIR/e2e/plexi-bin.sh"
EVID="${V1_ACCEPTANCE_EVID:-/tmp/plexi-v1-acceptance-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$EVID"

PR_NUM=""
BIN_ARG=""
TREE_ARG=""
ONLY="${V1_ACCEPTANCE_ONLY:-}"

usage() {
  cat <<'EOF'
usage: scripts/v1-acceptance.sh [--bin PATH] [--pr N] [--tree PATH]

  Default      this tree's target/release/plexi, copied to plexi-alpha.
               That is the alpha channel build. Not an integration branch.
  --bin PATH   channel binary override (a bare plexi is copied to plexi-alpha)
  --pr N       use plexi-pr-N from PATH (after just pr-install N)
  --tree PATH  tree whose item scripts and default-config.toml are used
               (default: the repo that contains this harness)

Env:
  PLEXI_BIN              same as --bin
  OPENROUTER_API_KEY     when set, the suite does not start the mock
  V1_ACCEPTANCE_ONLY     comma-separated item ids (V1-01,V1-14) for a partial run
  V1_ACCEPTANCE_EVID     directory for per-item logs (kept after the run)
  V1_ACCEPTANCE_TREE     same as --tree
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
    --tree)
      TREE_ARG="${2:-}"
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

if [[ -n "$TREE_ARG" ]]; then
  REPO="$(cd "$TREE_ARG" && pwd)"
elif [[ -n "${V1_ACCEPTANCE_TREE:-}" ]]; then
  REPO="$(cd "$V1_ACCEPTANCE_TREE" && pwd)"
fi
export PLEXI_E2E_SHIM="$SHIM"
# Skip-panes is a bypass. The suite never turns it on.
unset PLEXI_E2E_SKIP_PANES || true

# Caller pane vars would address the wrong host.
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT \
  PLEXI_CONTEXT_NAME PLEXI_RUNNING PLEXI_CALL_CREDENTIAL || true

REAL_HOME="${HOME}"
WORK_HOME="$(mktemp -d "${TMPDIR:-/tmp}/plexi-v1-home.XXXXXX")"
export HOME="$WORK_HOME"
mkdir -p "$HOME/.plexi"
# Python apps (chess, packaged samples) load the WASI bundle from the
# profile home. Item scripts symlink $ORIG_HOME/.plexi/wasm-bundles into
# their own temp home, and ORIG_HOME is this one.
if [[ -d /tmp/cpython-bundles ]]; then
  ln -sfn /tmp/cpython-bundles "$HOME/.plexi/wasm-bundles"
elif [[ -d "$REAL_HOME/.plexi/wasm-bundles" ]]; then
  ln -sfn "$REAL_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
# Never the login keychain. Channel scripts that need a keychain file inherit
# this; ones that create their own throwaway override it.
export PLEXI_KEYCHAIN_PATH="$HOME/v1-acceptance.keychain"
export PLEXI_KEYCHAIN_PASSWORD
PLEXI_KEYCHAIN_PASSWORD="$(openssl rand -hex 24 2>/dev/null || python3 -c 'import secrets; print(secrets.token_hex(24))')"
: >"$PLEXI_KEYCHAIN_PATH"
# Drop a caller's session bus. The preflight starts a private one under this
# HOME and unlocks gnome-keyring on it. Item scripts must not attach to the
# login secret service.
unset DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID || true

XVFB_PID=""
MOCK_PID=""
DISPLAY_NUM=""
KEYRING_PID=""
DOCKERD_PID=""
PREFLIGHT_PILLOW_ERR=""
PREFLIGHT_DOCKER_ERR=""
PREFLIGHT_SECRETS_ERR=""
PREFLIGHT_TESSERACT_ERR=""

cleanup() {
  if [[ -n "${REPO:-}" ]]; then
    find "$REPO/scripts" "$REPO/services" -name '.v1-shimmed-*.sh' -delete 2>/dev/null || true
  fi
  if [[ -n "${BIN:-}" && -x "${BIN:-}" ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$MOCK_PID" ]]; then
    kill "$MOCK_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$KEYRING_PID" ]]; then
    kill "$KEYRING_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "${DBUS_SESSION_BUS_PID:-}" ]]; then
    kill "$DBUS_SESSION_BUS_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$DOCKERD_PID" ]]; then
    sudo -n kill "$DOCKERD_PID" >/dev/null 2>&1 || kill "$DOCKERD_PID" >/dev/null 2>&1 || true
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

# Xvfb already running when the suite started, including this harness display.
# Item scripts are not allowed to leave another one behind.
BASELINE_XVFB_PIDS=" "
record_baseline_xvfb() {
  local pid
  BASELINE_XVFB_PIDS=" "
  if [[ -n "$XVFB_PID" ]]; then
    BASELINE_XVFB_PIDS+="$XVFB_PID "
  fi
  if ! command -v pgrep >/dev/null 2>&1; then
    return 0
  fi
  while read -r pid; do
    [[ -z "$pid" ]] && continue
    case " $BASELINE_XVFB_PIDS " in
      *" $pid "*) ;;
      *) BASELINE_XVFB_PIDS+="$pid " ;;
    esac
  done < <(pgrep -x Xvfb || true)
}
xvfb_is_baseline() {
  case " $BASELINE_XVFB_PIDS " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}
# kill -0 is true for a zombie. SIGKILL skips Xvfb's lock unlink, and a
# zombie still answers kill -0 while its socket is already closed.
proc_alive() {
  local state
  [[ -n "${1:-}" ]] || return 1
  kill -0 "$1" 2>/dev/null || return 1
  state="$(ps -o stat= -p "$1" 2>/dev/null || true)"
  [[ "$state" == Z* ]] && return 1
  return 0
}
# Drop X servers an item spawned, and the lock/socket of any display whose
# server is already gone. A live baseline server (the harness display, a
# display that was up before the suite) is left in place.
reap_item_xvfb() {
  local pid display holder lock
  if ! command -v pgrep >/dev/null 2>&1; then
    return 0
  fi
  while read -r pid; do
    [[ -z "$pid" ]] && continue
    if xvfb_is_baseline "$pid"; then
      continue
    fi
    echo "$(date -u +%H:%M:%S) reaped item Xvfb pid=$pid" >>"$EVID/xvfb-reap.log"
    kill "$pid" >/dev/null 2>&1 || true
  done < <(pgrep -x Xvfb || true)
  sleep 0.2
  while read -r pid; do
    [[ -z "$pid" ]] && continue
    if xvfb_is_baseline "$pid"; then
      continue
    fi
    kill -9 "$pid" >/dev/null 2>&1 || true
  done < <(pgrep -x Xvfb || true)
  shopt -s nullglob
  for lock in /tmp/.X*-lock; do
    display="${lock##*/}"
    display="${display#.X}"
    display="${display%-lock}"
    if [[ ":$display" == "${DISPLAY:-}" ]]; then
      continue
    fi
    holder="$(tr -dc '0-9' <"$lock" 2>/dev/null || true)"
    # The pid in a lock can be a recycled non-Xvfb process. Only the pgrep
    # loop above stops X servers. A live holder keeps its lock.
    if proc_alive "$holder"; then
      continue
    fi
    rm -f "$lock" "/tmp/.X11-unix/X${display}"
    echo "$(date -u +%H:%M:%S) removed stale X lock :$display" >>"$EVID/xvfb-reap.log"
  done
  shopt -u nullglob
}
record_baseline_xvfb

# Private runtime dir. The login XDG_RUNTIME_DIR is not this suite's bus,
# and gnome-keyring refuses a control socket it cannot own.
export XDG_RUNTIME_DIR="$WORK_HOME/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true
if [[ -f /usr/share/vulkan/icd.d/lvp_icd.json ]]; then
  export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
  export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
fi
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"

# Install the packages the preflight checks. One apt transaction. A missing
# sudo or a failed install leaves the later per-dep check to say ENV-FAIL.
preflight_apt() {
  local -a need=()
  if ! python3 -c 'import PIL' >/dev/null 2>&1; then
    need+=(python3-pil)
  fi
  if ! python3 -c 'import dbus' >/dev/null 2>&1; then
    need+=(python3-dbus)
  fi
  if ! command -v secret-tool >/dev/null 2>&1; then
    need+=(libsecret-tools)
  fi
  if ! command -v gnome-keyring-daemon >/dev/null 2>&1; then
    need+=(gnome-keyring)
  fi
  if ! command -v dbus-launch >/dev/null 2>&1; then
    need+=(dbus-x11)
  fi
  if ! command -v docker >/dev/null 2>&1; then
    need+=(docker.io)
  fi
  if ! command -v tesseract >/dev/null 2>&1; then
    need+=(tesseract-ocr tesseract-ocr-eng)
  fi
  if [[ ${#need[@]} -eq 0 ]]; then
    return 0
  fi
  if ! sudo -n true >/dev/null 2>&1; then
    echo "apt: sudo -n is not available; cannot install: ${need[*]}" >>"$EVID/preflight-apt.log"
    return 1
  fi
  if ! sudo -n DEBIAN_FRONTEND=noninteractive apt-get update -qq >>"$EVID/preflight-apt.log" 2>&1; then
    echo "apt: apt-get update failed" >>"$EVID/preflight-apt.log"
    return 1
  fi
  if ! sudo -n DEBIAN_FRONTEND=noninteractive apt-get install -y "${need[@]}" >>"$EVID/preflight-apt.log" 2>&1; then
    echo "apt: apt-get install failed: ${need[*]}" >>"$EVID/preflight-apt.log"
    return 1
  fi
  return 0
}

# gnome-keyring on this bus, login collection unlocked, no prompt.
# A nested dbus-run-session cannot activate org.freedesktop.secrets while
# that daemon is running, so the wrapper runs those item scripts here.
preflight_secrets() {
  local log="$EVID/preflight-secrets.log"
  : >"$log"
  if ! command -v gnome-keyring-daemon >/dev/null 2>&1 || ! command -v dbus-launch >/dev/null 2>&1; then
    PREFLIGHT_SECRETS_ERR="gnome-keyring or dbus-launch is not installed (log $log)"
    return 1
  fi
  if ! python3 -c 'import dbus' >/dev/null 2>&1 || ! command -v secret-tool >/dev/null 2>&1; then
    PREFLIGHT_SECRETS_ERR="python3-dbus or secret-tool is not installed (log $log)"
    return 1
  fi
  local launch
  launch="$(dbus-launch --sh-syntax 2>>"$log")" || {
    PREFLIGHT_SECRETS_ERR="dbus-launch failed (log $log)"
    return 1
  }
  # shellcheck disable=SC1090
  eval "$launch"
  if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    PREFLIGHT_SECRETS_ERR="dbus-launch did not set DBUS_SESSION_BUS_ADDRESS (log $log)"
    return 1
  fi
  export DBUS_SESSION_BUS_ADDRESS
  local pass
  pass="$(openssl rand -hex 16 2>/dev/null || python3 -c 'import secrets; print(secrets.token_hex(16))')"
  printf '%s' "$pass" | gnome-keyring-daemon --unlock --components=secrets --daemonize >>"$log" 2>&1 || true
  KEYRING_PID="$(pgrep -u "$(id -u)" -n -f '/usr/bin/gnome-keyring-daemon' || true)"
  local verify="" code=1 attempt
  for attempt in 1 2 3 4 5 6 7 8; do
    sleep 0.3
    verify="$(python3 - <<'PY' 2>>"$log"
import dbus, sys
bus = dbus.SessionBus()
proxy = bus.get_object("org.freedesktop.secrets", "/org/freedesktop/secrets")
svc = dbus.Interface(proxy, "org.freedesktop.Secret.Service")
default = str(svc.ReadAlias("default"))
session = str(svc.ReadAlias("session"))
print(f"default={default}")
print(f"session={session}")
if default in ("", "/"):
    sys.exit(2)
coll = bus.get_object("org.freedesktop.secrets", default)
locked = dbus.Interface(coll, "org.freedesktop.DBus.Properties").Get(
    "org.freedesktop.Secret.Collection", "Locked"
)
print(f"locked={int(bool(locked))}")
if int(bool(locked)) != 0:
    sys.exit(3)
output, session_path = svc.OpenSession("plain", dbus.String("", variant_level=1))
props = dbus.Dictionary({
    "org.freedesktop.Secret.Item.Label": "v1-acceptance-probe",
    "org.freedesktop.Secret.Item.Attributes": dbus.Dictionary({
        "xdg:schema": "com.plexi.HostSeal",
        "item": "v1-acceptance-probe",
    }, signature="ss"),
}, signature="sv")
secret = dbus.Struct((
    dbus.ObjectPath(session_path),
    dbus.ByteArray(b""),
    dbus.ByteArray(b"probe"),
    "text/plain",
))
item, prompt = dbus.Interface(coll, "org.freedesktop.Secret.Collection").CreateItem(props, secret, True)
print(f"prompt={prompt}")
if str(prompt) != "/":
    sys.exit(4)
print(f"item={item}")
PY
)"
    code=$?
    if [[ "$code" -eq 0 ]]; then
      break
    fi
  done
  printf '%s\n' "$verify" >>"$log"
  if [[ "$code" -ne 0 ]]; then
    PREFLIGHT_SECRETS_ERR="gnome-keyring did not unlock a login collection (exit $code, log $log)"
    return 1
  fi
  if ! printf 'probe-value' | secret-tool store --label='v1-acceptance-probe' service v1-acceptance account probe >>"$log" 2>&1; then
    PREFLIGHT_SECRETS_ERR="secret-tool store failed on the unlocked keyring (log $log)"
    return 1
  fi
  local got
  got="$(secret-tool lookup service v1-acceptance account probe 2>>"$log" || true)"
  if [[ "$got" != "probe-value" ]]; then
    PREFLIGHT_SECRETS_ERR="secret-tool lookup did not return the probe (log $log)"
    return 1
  fi
  secret-tool clear service v1-acceptance account probe >>"$log" 2>&1 || true
  mkdir -p "$EVID/bin"
  cat >"$EVID/bin/dbus-run-session" <<'EOF'
#!/usr/bin/env bash
# Acceptance preflight unlocked gnome-keyring on DBUS_SESSION_BUS_ADDRESS.
# A second session bus cannot activate org.freedesktop.secrets while that
# daemon is running, so journal and sealed-audit scripts stay on this bus.
# Scripts that unset the address before calling this wrapper still land here:
# the harness saved the unlocked bus in PLEXI_V1_DBUS_ADDRESS.
if [[ -n "${PLEXI_V1_SECRETS_READY:-}" ]]; then
  if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" && -n "${PLEXI_V1_DBUS_ADDRESS:-}" ]]; then
    export DBUS_SESSION_BUS_ADDRESS="$PLEXI_V1_DBUS_ADDRESS"
  fi
  if [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    if [[ "${1:-}" == "--" ]]; then
      shift
    fi
    exec "$@"
  fi
fi
exec /usr/bin/dbus-run-session "$@"
EOF
  chmod +x "$EVID/bin/dbus-run-session"
  export PATH="$EVID/bin:${PATH}"
  export PLEXI_V1_SECRETS_READY=1
  # Item scripts unset DBUS_SESSION_BUS_ADDRESS before calling dbus-run-session.
  # The wrapper restores this address so they stay on the unlocked keyring.
  export PLEXI_V1_DBUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
  return 0
}

preflight_secrets_cleanup_failed() {
  if [[ -n "$KEYRING_PID" ]]; then
    kill "$KEYRING_PID" >/dev/null 2>&1 || true
    KEYRING_PID=""
  fi
  if [[ -n "${DBUS_SESSION_BUS_PID:-}" ]]; then
    kill "$DBUS_SESSION_BUS_PID" >/dev/null 2>&1 || true
  fi
  unset DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID PLEXI_V1_SECRETS_READY || true
}

preflight_docker() {
  local log="$EVID/preflight-docker.log"
  : >"$log"
  if ! command -v docker >/dev/null 2>&1; then
    PREFLIGHT_DOCKER_ERR="docker is not installed (log $EVID/preflight-apt.log)"
    return 1
  fi
  if docker info >/dev/null 2>&1; then
    return 0
  fi
  # The package may be installed while the daemon is not running. This VM
  # has no systemd, so start dockerd directly. vfs is the fallback when the
  # default storage driver cannot mount inside a container.
  local driver
  for driver in "" vfs; do
    local -a args=(--host=unix:///var/run/docker.sock --iptables=false)
    if [[ -n "$driver" ]]; then
      args+=(--storage-driver="$driver")
    fi
    if [[ -n "$DOCKERD_PID" ]]; then
      sudo -n kill "$DOCKERD_PID" >/dev/null 2>&1 || true
      wait "$DOCKERD_PID" 2>/dev/null || true
      DOCKERD_PID=""
    fi
    sudo -n dockerd "${args[@]}" >>"$log" 2>&1 &
    DOCKERD_PID=$!
    local i
    for i in $(seq 1 40); do
      if [[ -S /var/run/docker.sock ]]; then
        sudo -n chmod 666 /var/run/docker.sock >>"$log" 2>&1 || true
      fi
      if docker info >/dev/null 2>&1; then
        return 0
      fi
      if ! kill -0 "$DOCKERD_PID" 2>/dev/null; then
        # dockerd often daemonizes and the sudo parent exits. A live socket
        # is still success.
        if docker info >/dev/null 2>&1; then
          return 0
        fi
        break
      fi
      sleep 0.5
    done
  done
  if docker info >/dev/null 2>&1; then
    return 0
  fi
  PREFLIGHT_DOCKER_ERR="dockerd did not become ready (log $log)"
  return 1
}

run_preflight() {
  preflight_apt || true
  if python3 -c 'from PIL import Image' >/dev/null 2>&1; then
    PREFLIGHT_PILLOW_ERR=""
  else
    PREFLIGHT_PILLOW_ERR="python3-pil is not importable (log $EVID/preflight-apt.log)"
  fi
  if preflight_docker; then
    PREFLIGHT_DOCKER_ERR=""
  fi
  if preflight_secrets; then
    PREFLIGHT_SECRETS_ERR=""
  else
    preflight_secrets_cleanup_failed
  fi
  if command -v tesseract >/dev/null 2>&1; then
    PREFLIGHT_TESSERACT_ERR=""
  else
    PREFLIGHT_TESSERACT_ERR="tesseract is not installed (log $EVID/preflight-apt.log)"
  fi
}

# A real file whose basename is the channel. current_exe() follows symlinks,
# so a symlink named plexi-alpha still reports the bare binary and writes
# ~/.plexi. A bare `plexi` adopts PLEXI_CHANNEL; acceptance copies it to
# plexi-alpha instead of setting that variable.
materialize_channel_bin() {
  local candidate="$1" base copy
  if [[ "$candidate" != /* ]]; then
    candidate="$(command -v "$candidate" 2>/dev/null || true)"
  fi
  if [[ -z "$candidate" || ! -x "$candidate" ]]; then
    printf ''
    return
  fi
  base="$(basename "$candidate")"
  base="${base%.exe}"
  mkdir -p "$EVID/bin"
  if [[ "$base" == "plexi" && -z "$PR_NUM" ]]; then
    copy="$EVID/bin/plexi-alpha"
    cp -f "$(readlink -f "$candidate")" "$copy"
    chmod +x "$copy"
    printf '%s' "$copy"
    return
  fi
  if [[ -L "$candidate" && "$base" == plexi-* ]]; then
    copy="$EVID/bin/$base"
    cp -f "$(readlink -f "$candidate")" "$copy"
    chmod +x "$copy"
    printf '%s' "$copy"
    return
  fi
  printf '%s' "$candidate"
}

resolve_bin() {
  local candidate=""
  if [[ -n "$BIN_ARG" ]]; then
    candidate="$BIN_ARG"
  elif [[ -n "$PR_NUM" ]]; then
    candidate="$(command -v "plexi-pr-${PR_NUM}" 2>/dev/null || true)"
    if [[ -z "$candidate" && -x "$REAL_HOME/.local/bin/plexi-pr-${PR_NUM}" ]]; then
      candidate="$REAL_HOME/.local/bin/plexi-pr-${PR_NUM}"
    fi
  elif [[ -n "${PLEXI_BIN:-}" ]]; then
    candidate="$PLEXI_BIN"
  elif [[ -x "$REPO/target/release/plexi" ]]; then
    # This tree's release build, named plexi-alpha. A plexi-alpha already on
    # PATH can be an older install or a copy of some other checkout.
    candidate="$REPO/target/release/plexi"
  elif command -v plexi-alpha >/dev/null 2>&1; then
    candidate="$(command -v plexi-alpha)"
  fi
  if [[ -z "$candidate" ]]; then
    printf ''
    return
  fi
  materialize_channel_bin "$candidate"
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
    plexi-alpha)
      unset PLEXI_CHANNEL || true
      # Item scripts derive PROFILE from CHANNEL. The alpha binary writes
      # ~/.plexi-alpha and ignores PLEXI_CHANNEL, so CHANNEL must be alpha.
      export CHANNEL=alpha
      ;;
    plexi-*)
      unset PLEXI_CHANNEL || true
      ;;
    plexi)
      if [[ -z "${PLEXI_CHANNEL:-}" ]]; then
        export PLEXI_CHANNEL="alpha"
      fi
      export CHANNEL="${CHANNEL:-alpha}"
      ;;
  esac
  export PATH="$(dirname "$BIN"):$PATH"
  export PLEXI_BIN="$BIN"
  export BIN
fi

# [ai] backend = "openrouter" from the tree's default config. Item scripts
# that point the broker at a local mock rewrite this file. Restoring the
# snapshot after every item keeps that rewrite from reaching the ledger.
CONFIG_BASELINE="$EVID/config-baseline.toml"
profile_config_path() {
  local base
  base="$(basename "${BIN:-plexi}")"
  base="${base%.exe}"
  if [[ "$base" == plexi-* ]]; then
    printf '%s/.%s/config.toml' "$HOME" "$base"
  elif [[ -n "${PLEXI_CHANNEL:-}" ]]; then
    printf '%s/.plexi-%s/config.toml' "$HOME" "$PLEXI_CHANNEL"
  else
    printf '%s/.plexi/config.toml' "$HOME"
  fi
}
seed_config_baseline() {
  local src path
  src="$REPO/scripts/default-config.toml"
  path="$(profile_config_path)"
  if [[ ! -f "$src" ]]; then
    return
  fi
  mkdir -p "$(dirname "$path")"
  cp -f "$src" "$path"
  cp -f "$src" "$CONFIG_BASELINE"
}
restore_config() {
  local path
  path="$(profile_config_path)"
  if [[ -f "$CONFIG_BASELINE" ]]; then
    if [[ -f "$path" ]] && ! cmp -s "$CONFIG_BASELINE" "$path"; then
      echo "$(date -u +%H:%M:%S) restored ${path} (item had rewritten config.toml)" >>"$EVID/config-restore.log"
    fi
    mkdir -p "$(dirname "$path")"
    cp -f "$CONFIG_BASELINE" "$path"
  fi
}
with_config() {
  # A previous item's host holds this profile's socket. Stop it before and
  # after so the next item starts its own host. Reap that item's Xvfb too:
  # xvfb-run -a leaves :99 with an auth cookie, xdpyinfo then reports the
  # display down, and the next script starts a host on a server it does not own.
  if [[ -n "${BIN:-}" && -x "$BIN" ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  reap_item_xvfb
  restore_config
  "$@"
  if [[ -n "${BIN:-}" && -x "$BIN" ]]; then
    "$BIN" host stop >/dev/null 2>&1 || true
  fi
  reap_item_xvfb
  restore_config
}

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
N_ENV=0

note_row() {
  local id="$1" status="$2" detail="$3"
  ROWS+=("$id"$'\t'"$status"$'\t'"$detail")
  printf '%s\t%s\t%s\n' "$id" "$status" "$detail"
  case "$status" in
    PASS) N_PASS=$((N_PASS + 1)) ;;
    FAIL)
      N_FAIL=$((N_FAIL + 1))
      if [[ "$detail" == bypass:* ]]; then
        N_BYPASS=$((N_BYPASS + 1))
      fi
      ;;
    ENV-FAIL) N_ENV=$((N_ENV + 1)) ;;
    NOT-LANDED) N_LANDED=$((N_LANDED + 1)) ;;
  esac
}

# A missing VM dependency is not a product failure. Skip the item and say
# which preflight check could not be made to work.
env_blocked() {
  local id="$1"
  local why=""
  case "$id" in
    V1-02)
      if [[ -n "$PREFLIGHT_PILLOW_ERR" ]]; then
        why="Pillow: $PREFLIGHT_PILLOW_ERR"
      fi
      ;;
    V1-08)
      if [[ -n "$PREFLIGHT_DOCKER_ERR" ]]; then
        why="Docker: $PREFLIGHT_DOCKER_ERR"
      fi
      ;;
    V1-04|V1-05|V1-06|V1-15)
      if [[ -n "$PREFLIGHT_SECRETS_ERR" ]]; then
        why="Secret Service: $PREFLIGHT_SECRETS_ERR"
      fi
      ;;
    V1-11)
      if [[ -n "$PREFLIGHT_TESSERACT_ERR" ]]; then
        why="tesseract: $PREFLIGHT_TESSERACT_ERR"
      elif [[ -n "$PREFLIGHT_SECRETS_ERR" ]]; then
        why="Secret Service: $PREFLIGHT_SECRETS_ERR"
      fi
      ;;
  esac
  if [[ -n "$why" ]]; then
    note_row "$id" "ENV-FAIL" "$why"
    return 0
  fi
  return 1
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

# driver-missing | human | none | bypass:<kinds>
# kinds are "skip panes", "CLI approval", and "env flag".
# A negative assertion (must not grant, permission_denied) is not CLI approval.
# The VERIFIED-VIA-BYPASS echo is CLI approval only when that branch ran, or
# the script grants from the CLI and never calls HUMAN_APPROVE.
script_bypass() {
  local script="$1"
  local log="$2"
  python3 - "$script" "$log" "$REPO/scripts/e2e/human.sh" <<'PY'
import re, sys
path, log_path, human = sys.argv[1:]
text = open(path, encoding="utf-8", errors="replace").read()
try:
    log = open(log_path, encoding="utf-8", errors="replace").read()
except FileNotFoundError:
    log = ""
human_ok = False
if __import__("os").path.isfile(human):
    human_ok = "HUMAN_APPROVE" in open(human, encoding="utf-8", errors="replace").read()
wants_human = "HUMAN_APPROVE" in text and re.search(r"human\.sh", text)
if wants_human and not human_ok:
    print("driver-missing")
    raise SystemExit(0)

lines = text.splitlines()
grant = re.compile(
    r"assistant permission resolve|needs-you resolve|permissions allow|"
    r"\bsecret grant\b|command-view resolve|\bcommand-view allow\b|\bchanges accept\b"
)
negative = (
    "must not grant",
    "did not grant",
    "does not record",
    "permission_denied",
    "must not approve",
    "did not approve",
    "not a grant",
    "not resolvable",
    "forged id",
    "are refused",
    "is refused",
)

def is_negative(idx, line):
    window = "\n".join(lines[max(0, idx - 3): idx + 12]).lower()
    if any(n in window for n in negative):
        return True
    if re.search(r"in_pane|\|\| true|\bgrep\b|\\\$PLEXI", line):
        return True
    if re.search(r"^\s*(pass|fail|ok|bad|record)\b", line):
        return True
    return False

def is_mention(line):
    # A skill lint or a regex names a refused command. It does not invoke it.
    if re.search(r"\bre\.(compile|search|match|findall|fullmatch)\b", line):
        return True
    if re.search(r"\b(pat|lint|refusal)\s*=", line) and "re." in line:
        return True
    return False

positive = []
for idx, raw in enumerate(lines):
    line = raw.split("#", 1)[0]
    if not grant.search(line):
        continue
    if is_mention(line) or is_negative(idx, line):
        continue
    # A raced deny settles the row as denied. It is not an approval.
    if re.search(r"needs-you resolve\b", line) and "--deny" in line and "--approve" not in line:
        continue
    positive.append(line.strip())

kinds = []
skip_taken = "SKIP: pane checks (PLEXI_E2E_SKIP_PANES=1)" in log or bool(
    re.search(r"(?m)^[ \t]*PLEXI_E2E_SKIP_PANES=1\b", text)
)
if skip_taken:
    kinds.append("skip panes")
env_flags = sorted(set(re.findall(r"\b(PLEXI_E2E_[A-Z0-9_]+)\b", text)))
env_flags = [name for name in env_flags if name != "PLEXI_E2E_SKIP_PANES"]
forced_env = []
for name in env_flags:
    if re.search(rf"(?m)^[ \t]*(?:export[ \t]+)?{name}=(?!\$)", text):
        forced_env.append(name)
if forced_env:
    kinds.append("env flag")
log_bypass = "VERIFIED-VIA-BYPASS" in log
log_human = (
    "HUMAN_APPROVE" in log
    or "human: click" in log
    or ("human: " in log and "resolved" in log)
)
if positive and not (wants_human and log_human and not log_bypass):
    if log_bypass and "VIA-BYPASS" not in kinds:
        kinds.append("VIA-BYPASS")
    if "CLI approval" not in kinds:
        kinds.append("CLI approval")
elif log_bypass and positive:
    if "VIA-BYPASS" not in kinds:
        kinds.append("VIA-BYPASS")
# A real click is a log line from human.sh. A script that only mentions
# HUMAN_APPROVE did not approve.
if wants_human and log_human and not kinds:
    print("human")
elif kinds:
    print("bypass:" + ", ".join(kinds))
else:
    print("none")
PY
}

script_needs_pr() {
  grep -qE '\$\{1:\?[^}]*<PR>' "$1"
}

# A <PR>-only script is copied beside itself with the shim source line in
# place of the PR/BIN pair, so dirname "$0" still finds the repo.
shim_script() {
  local script="$1"
  if grep -q 'PLEXI_E2E_SHIM\|plexi-bin.sh' "$script"; then
    printf '%s' "$script"
    return
  fi
  if ! script_needs_pr "$script"; then
    printf '%s' "$script"
    return
  fi
  local dest
  dest="$(dirname "$script")/.v1-shimmed-$(basename "$script")"
  python3 - "$script" "$dest" <<'PY'
import sys
src, dest = sys.argv[1:]
lines = open(src, encoding="utf-8", errors="replace").read().splitlines(True)
out = []
i = 0
replaced = False
while i < len(lines):
    if (not replaced) and "${1:?" in lines[i] and "<PR>" in lines[i]:
        out.append('source "${PLEXI_E2E_SHIM:?}"\n')
        i += 1
        if i < len(lines) and "BIN=" in lines[i] and "plexi-pr-" in lines[i]:
            i += 1
        replaced = True
        continue
    out.append(lines[i])
    i += 1
if not replaced:
    raise SystemExit("shim: no <PR> line in " + src)
open(dest, "w", encoding="utf-8").writelines(out)
PY
  chmod +x "$dest"
  printf '%s' "$dest"
}

invoke_script() {
  local script="$1"
  local log="$2"
  export PLEXI_BIN="$BIN"
  export PLEXI_E2E_SHIM="$SHIM"
  export BIN
  local run="$script"
  local shimmed=0
  if script_needs_pr "$script"; then
    run="$(shim_script "$script")"
    if [[ "$run" != "$script" ]]; then
      shimmed=1
    fi
  fi
  local -a cmd
  if grep -q 'path-to-plexi-binary' "$script"; then
    cmd=(timeout --foreground 1200 bash "$run" "$BIN")
  else
    cmd=(timeout --foreground 1200 bash "$run")
  fi
  set +e
  "${cmd[@]}" >"$log" 2>&1
  local code=$?
  set +e
  if [[ "$shimmed" -eq 1 ]]; then
    rm -f "$run"
  fi
  local mode
  mode="$(script_bypass "$script" "$log")"
  if [[ "$mode" == "driver-missing" ]]; then
    echo "driver-missing"
    return 0
  fi
  if [[ "$code" -eq 0 ]]; then
    echo "$mode"
  else
    echo "fail:$code:$mode"
  fi
}

# Run one or more scripts. A required path that is missing → NOT-LANDED.
# Paths after --any are alternates (a later PR renamed the script): the item
# is landed when any one of them exists, and every one that exists is run.
# Paths after --optional run when present and are never required.
# requires_human=1: a green run that never called HUMAN_APPROVE is FAIL
# (bypass: none). A named bypass is FAIL on every item.
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
  # An alternate whose body is `exec` of another alternate is a rename stub.
  # Running it skips the <PR> shim and fails before the real script.
  local -a alt_present=()
  local other base wrapped
  for path in "${alternates[@]}"; do
    [[ -f "$REPO/$path" ]] || continue
    wrapped=0
    for other in "${alternates[@]}"; do
      [[ "$other" == "$path" ]] && continue
      base="$(basename "$other")"
      if grep -qE "exec[[:space:]].*${base}" "$REPO/$path"; then
        wrapped=1
        break
      fi
    done
    if [[ "$wrapped" -eq 0 ]]; then
      alt_present+=("$path")
    fi
  done
  if [[ ${#alt_present[@]} -eq 0 ]]; then
    for path in "${alternates[@]}"; do
      if [[ -f "$REPO/$path" ]]; then
        alt_present+=("$path")
      fi
    done
  fi
  for path in "${alt_present[@]}"; do
    present+=("$path")
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

  local saw_human=0
  local bypass_kinds="" detail="" path mode log rest code kind
  for path in "${present[@]}"; do
    log="$EVID/${id}-$(basename "$path").log"
    mode="$(invoke_script "$REPO/$path" "$log")"
    case "$mode" in
      driver-missing)
        note_row "$id" "NOT-LANDED" "$path requires scripts/e2e/human.sh (W15), which is not on this tree"
        return
        ;;
      fail:*)
        rest="${mode#fail:}"
        code="${rest%%:*}"
        kind="${rest#*:}"
        if [[ "$kind" == bypass:* ]]; then
          note_row "$id" "FAIL" "bypass: ${kind#bypass:} — $path exited $code (log $log)"
        else
          note_row "$id" "FAIL" "$path exited $code (log $log)"
        fi
        return
        ;;
      human) saw_human=1; detail+=" $path" ;;
      none) detail+=" $path" ;;
      bypass:*)
        if [[ -n "$bypass_kinds" ]]; then
          bypass_kinds+="; "
        fi
        bypass_kinds+="${mode#bypass:} ($path)"
        detail+=" $path"
        ;;
      *)
        note_row "$id" "FAIL" "$path returned unexpected status $mode"
        return
        ;;
    esac
  done

  if [[ -n "$bypass_kinds" ]]; then
    note_row "$id" "FAIL" "bypass: ${bypass_kinds} — ran${detail}; see scripts/e2e/plexi-bin.md"
    return
  fi
  if [[ "$requires_human" -eq 1 && "$saw_human" -eq 0 ]]; then
    note_row "$id" "FAIL" "bypass: none — ran${detail}; contract requires HUMAN_APPROVE and the script never clicks (scripts/e2e/plexi-bin.md)"
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
        rest="${check_mode#fail:}"
        code="${rest%%:*}"
        kind="${rest#*:}"
        if [[ "$kind" == bypass:* ]]; then
          note_row "$id" "FAIL" "bypass: ${kind#bypass:} — scripts/skill-check.sh exited $code (log $check_log)"
        else
          note_row "$id" "FAIL" "scripts/skill-check.sh exited $code (log $check_log)"
        fi
        return
        ;;
      driver-missing)
        note_row "$id" "NOT-LANDED" "scripts/skill-check.sh could not run ($check_mode)"
        return
        ;;
      bypass:*)
        note_row "$id" "FAIL" "bypass: ${check_mode#bypass:} — scripts/skill-check.sh (scripts/e2e/plexi-bin.md)"
        return
        ;;
      human|none)
        # #2722's skill-check is the contract check. The inline lint below
        # flags skill lines that name a refused command, which that script
        # already accepts.
        local summary
        summary="$(grep -E '^PASS' "$check_log" | tail -1)"
        note_row "$id" "PASS" "${summary:-scripts/skill-check.sh passed}"
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
  if env_blocked "$id"; then
    return
  fi
  run_scripts "$id" "$id" "$requires_human" "${rest[@]}"
}

run_preflight

echo "v1-acceptance"
echo "binary    ${BIN:-<none>} (${BIN_BASE:-})"
echo "channel   ${BIN_BASE:-<none>}"
if [[ -n "$BIN" ]]; then
  echo "version   $("$BIN" --version 2>&1 | grep -E 'plexi [0-9]' | head -1)"
fi
echo "tree      $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo "display   $DISPLAY"
echo "home      $HOME"
echo "keychain  $PLEXI_KEYCHAIN_PATH"
echo "openrouter $OPENROUTER_MODE"
echo "evidence  $EVID"
seed_config_baseline
echo "config    $(profile_config_path) (restored between items)"
if [[ -z "$PREFLIGHT_PILLOW_ERR" ]]; then
  echo "preflight pillow ok"
else
  echo "preflight pillow ENV-FAIL $PREFLIGHT_PILLOW_ERR"
fi
if [[ -z "$PREFLIGHT_DOCKER_ERR" ]]; then
  echo "preflight docker ok"
else
  echo "preflight docker ENV-FAIL $PREFLIGHT_DOCKER_ERR"
fi
if [[ -z "$PREFLIGHT_SECRETS_ERR" ]]; then
  echo "preflight secret-service ok (gnome-keyring login unlocked)"
else
  echo "preflight secret-service ENV-FAIL $PREFLIGHT_SECRETS_ERR"
fi
if [[ -z "$PREFLIGHT_TESSERACT_ERR" ]]; then
  echo "preflight tesseract ok"
else
  echo "preflight tesseract ENV-FAIL $PREFLIGHT_TESSERACT_ERR"
fi
echo

with_config item_v1_01
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
with_config run_one V1-02 1 --probe assistant permission -- scripts/permission-gate-e2e.sh
with_config run_one V1-03 1 -- scripts/no-self-approval-e2e.sh
with_config run_one V1-04 1 --probe permissions -- scripts/permissions-seal-e2e.sh
with_config run_one V1-05 1 --probe needs-you -- scripts/needs-you-e2e.sh scripts/needs-you-persist-e2e.sh
with_config run_one V1-06 1 -- scripts/folder-secrets-e2e.sh
with_config run_one V1-07 0 --probe ledger -- scripts/e2e/ledger/run.sh
with_config run_one V1-08 1 -- services/relay/e2e_installed.sh
with_config run_one V1-09 1 --probe agent head -- scripts/e2e_agents_api_installed.sh
with_config run_one V1-10 1 --probe agent head -- scripts/multi-lead-e2e.sh scripts/headless-queue-e2e.sh
with_config run_one V1-11 1 --probe command-view -- --any scripts/command-view-steer-e2e.sh scripts/command-view-e2e.sh
with_config run_one V1-12 1 --probe changes -- --any scripts/change-sets-e2e.sh scripts/assistant-editor-change-set-e2e.sh
with_config run_one V1-13 0 -- scripts/cloud-basics-e2e.sh
with_config item_v1_14
with_config run_one V1-15 1 --probe app package -- scripts/app-share-e2e.sh

echo

verdict="PASS"
exit_code=0
if [[ "$N_FAIL" -gt 0 ]]; then
  verdict="FAIL"
  exit_code=1
elif [[ "$N_ENV" -gt 0 ]]; then
  verdict="ENV-FAIL"
  exit_code=3
elif [[ "$N_LANDED" -gt 0 ]]; then
  verdict="INCOMPLETE"
  exit_code=2
fi
echo "VERDICT ${verdict}  pass=${N_PASS} fail=${N_FAIL} not-landed=${N_LANDED} bypass-fail=${N_BYPASS} env-fail=${N_ENV}"
exit "$exit_code"

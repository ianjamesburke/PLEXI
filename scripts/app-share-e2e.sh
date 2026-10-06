#!/usr/bin/env bash
# Installed-binary check: package a sample app, install it, and confirm the
# one grant store. A declared non-sensitive capability is auto-granted and
# listed. A sensitive capability is absent until a human grants it. Deleting
# permissions.toml does not change the list.
set -euo pipefail

# Auto-grant has to seal grants.toml. Linux can do that only with Secret Service.
if [[ "$(uname -s)" == "Linux" && -z "${APP_SHARE_E2E_INNER:-}" ]] && command -v dbus-run-session >/dev/null 2>&1; then
  exec dbus-run-session -- env APP_SHARE_E2E_INNER=1 "$0" "$@"
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${PLEXI_BIN:-$ROOT/target/release/plexi}"
if [[ ! -x "$BIN" ]]; then
  echo "error: missing binary $BIN (run just build first)" >&2
  exit 1
fi

# A channel-named binary ignores PLEXI_CHANNEL. Match its profile dir.
bin_base="$(basename "$BIN")"
bin_base="${bin_base%.exe}"
if [[ "$bin_base" == plexi-* ]]; then
  export PLEXI_CHANNEL="${bin_base#plexi-}"
else
  export PLEXI_CHANNEL="${PLEXI_CHANNEL:-app-share-e2e}"
fi

WORK="$(mktemp -d)"
trap 'if [[ -n "${HOST_PID:-}" ]]; then kill -- "-$HOST_PID" 2>/dev/null || kill "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; fi; rm -rf "$WORK"' EXIT

ORIG_HOME="${HOME}"
export HOME="$WORK/home"
mkdir -p "$HOME/.plexi"
if [[ -d "$ORIG_HOME/.plexi/wasm-bundles" ]]; then
  ln -s "$ORIG_HOME/.plexi/wasm-bundles" "$HOME/.plexi/wasm-bundles"
fi
unset PLEXI_SOCKET PLEXI_PANE_ID PLEXI_CONTEXT_ID PLEXI_CONTEXT_ROOT PLEXI_RUNNING || true
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export XDG_RUNTIME_DIR="$WORK/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true

PROFILE="$HOME/.plexi-$PLEXI_CHANNEL"
SOCKET="$PROFILE/notify.sock"
APP_ID="sample-share"

start_host() {
  unset DISPLAY
  if command -v xvfb-run >/dev/null 2>&1; then
    setsid xvfb-run -a "$BIN" >"$WORK/host.log" 2>&1 &
  else
    setsid "$BIN" >"$WORK/host.log" 2>&1 &
  fi
  HOST_PID=$!
  for _ in $(seq 1 90); do
    if [[ -S "$SOCKET" ]]; then
      return 0
    fi
    if ! kill -0 "$HOST_PID" 2>/dev/null; then
      echo "error: host exited before the socket appeared" >&2
      cat "$WORK/host.log" >&2 || true
      return 1
    fi
    sleep 1
  done
  echo "error: notify socket did not appear at $SOCKET" >&2
  cat "$WORK/host.log" >&2 || true
  return 1
}

stop_host() {
  if [[ -n "${HOST_PID:-}" ]]; then
    kill -- "-$HOST_PID" 2>/dev/null || kill "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
    unset HOST_PID
  fi
  rm -f "$SOCKET"
}

echo "starting host $BIN"
start_host
export PLEXI_SOCKET="$SOCKET"

echo "scaffolding $APP_ID"
"$BIN" app init "$APP_ID" --global >"$WORK/init.log" 2>&1
APP_DIR="$PROFILE/apps/$APP_ID"
if [[ ! -f "$APP_DIR/manifest.toml" ]]; then
  echo "error: scaffold did not write $APP_DIR/manifest.toml" >&2
  cat "$WORK/init.log" >&2
  exit 1
fi
python3 - "$APP_DIR/manifest.toml" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = 'capabilities = ["timer"]'
new = 'capabilities = ["timer", "permissions.manage"]'
if old not in text:
    sys.exit(f"scaffold manifest has no timer capability list:\n{text}")
path.write_text(text.replace(old, new, 1))
PY

echo "validating and packaging"
"$BIN" app validate "$APP_DIR" >"$WORK/validate.log" 2>&1
"$BIN" app package "$APP_DIR" --out "$WORK/$APP_ID.plexipkg" >"$WORK/package.log" 2>&1
test -f "$WORK/$APP_ID.plexipkg"

echo "installing package"
"$BIN" app install "$WORK/$APP_ID.plexipkg" --yes >"$WORK/install.log" 2>&1
if ! grep -q timer "$WORK/install.log"; then
  echo "error: install trust sheet did not name timer" >&2
  cat "$WORK/install.log" >&2
  exit 1
fi

echo "opening app"
if ! "$BIN" app open "$APP_ID" >"$WORK/open.log" 2>&1; then
  echo "warning: app open returned non-zero; the grant check still decides" >&2
  cat "$WORK/open.log" >&2 || true
fi

echo "waiting for permission rows"
ready=0
for _ in $(seq 1 40); do
  if "$BIN" permissions list --json >"$WORK/list.json" 2>"$WORK/list.err"; then
    if python3 - "$WORK/list.json" <<'PY'
import json, sys
rows = json.load(open(sys.argv[1]))
items = rows if isinstance(rows, list) else rows.get("entries") or rows.get("permissions") or []
blob = json.dumps(items)
ok = "timer" in blob and "permissions.manage" not in blob
sys.exit(0 if ok else 1)
PY
    then
      ready=1
      break
    fi
  fi
  sleep 1
done
if [[ "$ready" != 1 ]]; then
  echo "error: permissions list did not show timer without permissions.manage" >&2
  cat "$WORK/list.json" "$WORK/list.err" "$WORK/open.log" >&2 || true
  exit 1
fi
cp "$WORK/list.json" "$WORK/list-before.json"
echo "list shows the auto-granted timer row and withholds permissions.manage"

echo "removing permissions.toml"
rm -f "$PROFILE/permissions.toml"
stop_host
start_host
export PLEXI_SOCKET="$SOCKET"
ready=0
for _ in $(seq 1 40); do
  if "$BIN" permissions list --json >"$WORK/list-after.json" 2>"$WORK/list-after.err"; then
    if grep -q timer "$WORK/list-after.json"; then
      ready=1
      break
    fi
  fi
  sleep 1
done
python3 - "$WORK/list-before.json" "$WORK/list-after.json" <<'PY'
import json, sys
def items(path):
    rows = json.load(open(path))
    return rows if isinstance(rows, list) else rows.get("entries") or rows.get("permissions") or []
before = json.dumps(items(sys.argv[1]), sort_keys=True)
after = json.dumps(items(sys.argv[2]), sort_keys=True)
if "timer" not in after or "permissions.manage" in after:
    sys.exit(f"enforcement changed after deleting permissions.toml:\n{after[:800]}")
if before != after:
    sys.exit(f"permission list changed after deleting permissions.toml\nbefore={before[:400]}\nafter={after[:400]}")
print("deleting permissions.toml changed nothing")
PY

if [[ -f "$PROFILE/grants.toml" ]]; then
  if ! grep -q timer "$PROFILE/grants.toml"; then
    echo "error: grants.toml has no timer row" >&2
    exit 1
  fi
  echo "grants.toml contains the timer grant"
else
  echo "error: grants.toml was not written" >&2
  exit 1
fi

echo "app-share-e2e passed"

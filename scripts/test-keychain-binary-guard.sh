#!/usr/bin/env bash
# Only inert executables are supplied to entry points. Record any attempt to
# launch a test workload, start scratch setup, or contact the Keychain.
set -euo pipefail
scripts="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
work="$(cd "$work" && pwd -P)"
trap 'rm -rf "$work"' EXIT
clean="$work/target/release/plexi"
valid="$work/A/plexi-pr-42"
mkdir -p "$work"/{target/release,bin,A,B,home,tmp}
cat >"$clean" <<'SH'
#!/bin/sh
echo "$0" >> "$CALLS"
exit 99
SH
chmod +x "$clean"
for command in security mktemp mkdir Xvfb dbus-run-session; do
  ln -s "$clean" "$work/bin/$command"
done
export CALLS="$work/calls"
export PATH="$work/bin:$PATH" HOME="$work/home" TMPDIR="$work/tmp"
export V1_ACCEPTANCE_EVID="$work/evidence" PYTHONDONTWRITEBYTECODE=1

# Catch the Python paths that bypass shell utilities, even if a regression
# moves SecKeychainCreate or TemporaryDirectory ahead of the security shim.
cat >"$work/sitecustomize.py" <<'PY'
import os
import sys

def guard(event, args):
    if event == "tempfile.mkdtemp" or (event == "ctypes.dlopen" and "Security" in str(args[0])):
        with open(os.environ["CALLS"], "a") as log:
            print(event, file=log)
        raise RuntimeError(f"unexpected {event} during preflight")

sys.addaudithook(guard)
PY
export PYTHONPATH="$work"

refused() {
  local binary="$1" expected_code="$2" reason="$3" status=0
  shift 3
  "$@" >"$work/out" 2>&1 || status=$?
  if [[ $status != "$expected_code" ]] ||
     ! grep -Fq -- "Refused $binary:" "$work/out" ||
     ! grep -Fq -- "$reason" "$work/out" ||
     ! grep -Fq 'PLEXI_BUILD_TEST_CHANNEL=pr-<N> release build' "$work/out" ||
     ! grep -Fq 'install it as plexi-pr-<N>' "$work/out"; then
    cat "$work/out" >&2
    echo "FAIL: expected refusal (exit $expected_code), got $status from $*" >&2
    exit 1
  fi
  if [[ -e "$CALLS" ]]; then
    cat "$CALLS" >&2
    exit 1
  fi
}

helper=(python3 "$scripts/require_test_hook_binary.py")
refused "$clean" 1 missing "${helper[@]}" "$clean"
refused "$work/absent" 2 readable "${helper[@]}" "$work/absent"
bash "$scripts/check-release-hook-names.sh" "$clean"

# Build independent fixtures from the actual declarations the host compiles.
names=()
while IFS= read -r name; do names+=("$name"); done < <(
  sed -n 's/.*= hook!("\([A-Z_]*\)");/PLEXI_\1/p' "$scripts/../src/config/test_hooks.rs"
)
[[ ${#names[@]} == 4 ]]
for missing in "${names[@]}"; do
  printf '\377\0' >"$work/partial"
  for name in "${names[@]}"; do
    if [[ "$name" != "$missing" ]]; then printf '%s\0' "$name" >>"$work/partial"; fi
  done
  refused "$work/partial" 1 "$missing" "${helper[@]}" "$work/partial"
done

cat >"$valid" <<'SH'
#!/bin/sh
if [ "$#" = 1 ] && [ "$1" = --build-info ]; then
  echo '{"test_channel":"pr-42"}'
  exit 0
fi
echo "$0" >> "$CALLS"
exit 99
SH
printf '\377\0%s\0' "${names[@]}" >>"$valid"
chmod +x "$valid"
"${helper[@]}" "$valid"
"${helper[@]}" --installed "$valid" >/dev/null
cp "$valid" "$work/A/plexi"
refused "$work/A/plexi" 1 'numbered PR executable' "${helper[@]}" --installed "$work/A/plexi"
cp "$valid" "$work/A/plexi-pr-43"
refused "$work/A/plexi-pr-43" 1 'does not match' "${helper[@]}" --installed "$work/A/plexi-pr-43"
ln -s "$valid" "$work/B/plexi-pr-43"
refused "$work/B/plexi-pr-43" 1 'same basename' "${helper[@]}" --installed "$work/B/plexi-pr-43"
# An installed `plexi-pr-<N>` on PATH is a generated launcher; the executable
# its receipt names is what gets judged.
/bin/mkdir -p "$work/L" "$work/R"
printf '#!/bin/sh\nexec %s launch --receipt %s -- "$@"\n' "'/x/plexi-installer'" "'$work/R'" >"$work/L/plexi-pr-42"
chmod +x "$work/L/plexi-pr-42"
printf '{"active":{"path":"%s","executable":"plexi-pr-42"}}' "$work/A" >"$work/R/installation.json"
[[ "$("${helper[@]}" --installed "$work/L/plexi-pr-42")" == "$work/A/plexi-pr-42" ]]
/bin/mkdir -p "$work/C"
cp "$clean" "$work/C/plexi-pr-42"
printf '{"active":{"path":"%s","executable":"plexi-pr-42"}}' "$work/C" >"$work/R/installation.json"
refused "$work/C/plexi-pr-42" 1 missing "${helper[@]}" --installed "$work/L/plexi-pr-42"
if bash "$scripts/check-release-hook-names.sh" "$valid" >"$work/out" 2>&1; then
  echo 'FAIL: release scan accepted test hooks' >&2
  exit 1
fi
echo 'PASS: byte scans, each missing hook, unreadable file and runtime channel checks'

export PLEXI_BIN="$clean"
for script in folder-secrets-e2e.sh permissions-seal-e2e.sh change-sets-e2e.sh v1-acceptance.sh; do
  refused "$clean" 1 missing bash "$scripts/$script"
  echo "PASS: $script refused before setup"
done
refused "$clean" 1 missing python3 "$scripts/secret-inventory-e2e.py" "$clean"
echo 'PASS: secret-inventory-e2e.py refused before setup'

cp "$clean" "$work/B/plexi-pr-42"
for package in A B; do
  echo '{"channel":"pr-42","executable":"plexi-pr-42"}' >"$work/$package/package.json"
done
bad="$work/B/plexi-pr-42"
refused "$bad" 1 missing python3 "$scripts/test-uninstall-data.py" --package "$work/B" --installer "$clean"
for first in B A; do
  refused "$bad" 1 missing python3 "$scripts/test-macos-keychain-continuity.py" --package-a "$work/$first" --package-b "$work/B"
done
echo 'PASS: test-uninstall-data.py and both test-macos-keychain-continuity.py package positions refused before setup'
[[ ! -e "$CALLS" && ! -e "$V1_ACCEPTANCE_EVID" ]]
echo 'PASS: zero Keychain, setup or workload calls'

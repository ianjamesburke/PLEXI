#!/usr/bin/env bash
# Focused release-installer failure harness. It never contacts the network.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fake_bin="$work/bin"
mkdir -p "$fake_bin"
mkdir -p "$work/package"
printf '#!/bin/sh\necho fixture\n' > "$work/package/plexi"
chmod +x "$work/package/plexi"
tar -C "$work/package" -czf "$work/release.tar.gz" plexi

cat > "$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
case "${INSTALL_FIXTURE:-}" in
  missing-asset) exit 22 ;;
  checksum-mismatch)
    if [[ "$url" == *.sha256 ]]; then
      printf '%064d  plexi-linux-x64.tar.gz\n' 0 > "$output"
    else
      printf 'not the expected archive' > "$output"
    fi
    ;;
  activation-failure)
    if [[ "$url" == *.sha256 ]]; then
      sha256sum "$INSTALL_ARCHIVE" | sed 's#  .*#  plexi-linux-x64.tar.gz#' > "$output"
    else
      cp "$INSTALL_ARCHIVE" "$output"
    fi
    ;;
  *) echo "unexpected fixture: ${INSTALL_FIXTURE:-}" >&2; exit 99 ;;
esac
EOF
chmod +x "$fake_bin/curl"

cat > "$fake_bin/mv" <<'EOF'
#!/usr/bin/env bash
if [[ "${INSTALL_FAIL_BINARY_SWAP:-}" == 1 && "$1" == */staged/plexi-alpha && "$2" == */bin-install/plexi-alpha ]]; then
  echo 'forced command activation failure' >&2
  exit 1
fi
exec /bin/mv "$@"
EOF
chmod +x "$fake_bin/mv"

assert_absent() {
  [[ ! -e "$1" ]] || { echo "expected absent: $1" >&2; exit 1; }
}

run_installer() {
  local fixture="$1"; shift
  local fail_binary_swap=0
  [[ "$fixture" == activation-failure ]] && fail_binary_swap=1
  set +e
  INSTALL_FIXTURE="$fixture" INSTALL_ARCHIVE="$work/release.tar.gz" INSTALL_FAIL_BINARY_SWAP="$fail_binary_swap" PATH="$fake_bin:$PATH" HOME="$work/home" \
    PLEXI_INSTALL_DIR="$work/install" PLEXI_BIN_DIR="$work/bin-install" \
    PLEXI_RELEASE_BASE_URL="https://fixture.invalid/releases" \
    bash "$repo_root/scripts/install.sh" "$@" >"$work/output" 2>&1
  local status=$?
  set -e
  [[ $status -ne 0 ]] || { echo "installer unexpectedly succeeded" >&2; cat "$work/output" >&2; exit 1; }
  ! rg -q '^Installed ' "$work/output" || { echo "installer printed false success" >&2; cat "$work/output" >&2; exit 1; }
}

# Invalid input fails before any network or profile mutation.
run_installer missing-asset --channel bad
[[ $(<"$work/output") == *'channel must be main, alpha, or beta'* ]]
assert_absent "$work/install"
assert_absent "$work/home/.plexi/installed_tag"

# A failed download leaves a prior install and its marker untouched.
mkdir -p "$work/install/alpha" "$work/bin-install" "$work/home/.plexi-alpha"
printf 'old payload' > "$work/install/alpha/keep"
printf 'old command' > "$work/bin-install/plexi-alpha"
printf 'v0.0.1-alpha.1\n' > "$work/home/.plexi-alpha/installed_tag"
run_installer missing-asset --channel alpha --tag v0.0.2-alpha.1
[[ $(<"$work/install/alpha/keep") == 'old payload' ]]
[[ $(<"$work/bin-install/plexi-alpha") == 'old command' ]]
[[ $(<"$work/home/.plexi-alpha/installed_tag") == 'v0.0.1-alpha.1' ]]

# A downloaded but corrupted archive is rejected before the live paths move.
run_installer checksum-mismatch --channel alpha --tag v0.0.2-alpha.1
[[ $(<"$work/install/alpha/keep") == 'old payload' ]]
[[ $(<"$work/bin-install/plexi-alpha") == 'old command' ]]
[[ $(<"$work/home/.plexi-alpha/installed_tag") == 'v0.0.1-alpha.1' ]]
rg -q 'SHA-256 mismatch' "$work/output"

# A failure while activating the staged command rolls back the already-swapped
# payload and restores the old marker instead of leaving a half-install.
run_installer activation-failure --channel alpha --tag v0.0.2-alpha.1
[[ $(<"$work/install/alpha/keep") == 'old payload' ]]
[[ $(<"$work/bin-install/plexi-alpha") == 'old command' ]]
[[ $(<"$work/home/.plexi-alpha/installed_tag") == 'v0.0.1-alpha.1' ]]
rg -q 'restoring the previous install' "$work/output"

echo "release installer failure harness: PASS"

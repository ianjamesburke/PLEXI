#!/usr/bin/env bash
# Versioned binary bootstrap. The release installer owns selection and activation.
set -euo pipefail
case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) echo 'Use https://plexiapp.com/install.ps1 from PowerShell on Windows.' >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64) arch=x64 ;;
  *) echo 'Unsupported processor architecture.' >&2; exit 1 ;;
esac
base="${PLEXI_RELEASE_BASE_URL:-https://github.com/ianjamesburke/PLEXI/releases/download}"
# Resolve once, then pin both executable and checksum to the same released tag.
tag="${PLEXI_BOOTSTRAP_TAG:-}"
if [[ -z "$tag" ]]; then
  metadata="$(curl -fsSL --connect-timeout 15 "${PLEXI_RELEASES_URL:-https://api.github.com/repos/ianjamesburke/PLEXI/releases}/latest")"
  tag="$(printf '%s' "$metadata" | sed -nE 's/.*"tag_name"[[:space:]]*:[[:space:]]*"(v[0-9]+\.[0-9]+\.[0-9]+)".*/\1/p' | head -1)"
fi
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-alpha\.[0-9]+|-beta\.[0-9]+)?$ ]] || { echo 'No valid released installer tag was found.' >&2; exit 1; }
asset="plexi-installer-${os}-${arch}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fL --retry 3 --connect-timeout 15 "$base/$tag/$asset" -o "$work/$asset"
curl -fL --retry 3 --connect-timeout 15 "$base/$tag/$asset.sha256" -o "$work/checksum"
expected="$(awk -v name="$asset" '$2 == name || $2 == "*" name {print $1}' "$work/checksum")"
[[ "$expected" =~ ^[[:xdigit:]]{64}$ ]] || { echo 'Malformed installer checksum.' >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then actual="$(sha256sum "$work/$asset" | awk '{print $1}')"
else actual="$(shasum -a 256 "$work/$asset" | awk '{print $1}')"; fi
[[ "$actual" == "$expected" ]] || { echo 'Installer checksum mismatch.' >&2; exit 1; }
chmod 755 "$work/$asset"
"$work/$asset" "$@"

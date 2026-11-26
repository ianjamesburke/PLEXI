#!/usr/bin/env bash
# Source and download installs feed the same package transaction.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${1:-}" != --from-source ]]; then
  exec bash "$repo/install.sh" "$@"
fi
shift
cd "$repo"
# Optional channel, then installer flags (e.g. --trash-strays) passed through.
channel=""
if [[ $# -gt 0 && "$1" != --* ]]; then channel="$1"; shift; fi
if [[ -z "$channel" ]]; then
  branch="$(git branch --show-current)"
  case "$branch" in main|alpha|beta) channel="$branch" ;; *) channel="$(cat .channel 2>/dev/null || echo alpha)" ;; esac
fi
case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) echo 'Source packaging requires macOS or Linux.' >&2; exit 1 ;; esac
case "$(uname -m)" in arm64|aarch64) arch=arm64 ;; x86_64) arch=x64 ;; *) echo 'Unsupported architecture.' >&2; exit 1 ;; esac
if [[ "$channel" =~ ^pr-[0-9]+$ ]]; then export PLEXI_BUILD_TEST_CHANNEL="$channel"; else unset PLEXI_BUILD_TEST_CHANNEL; fi
if [[ "$channel" == alpha && -z "${PLEXI_BUILD_TAG:-}" ]]; then
  PLEXI_BUILD_TAG="$(bash scripts/local-alpha-tag.sh)"
  export PLEXI_BUILD_TAG
  echo "install: local alpha build $PLEXI_BUILD_TAG"
fi
metadata="$(cargo metadata --format-version=1 --no-deps)"
target="$(printf '%s' "$metadata" | uv run --no-project --python 3.11 python -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
just build
if [[ -z "${PLEXI_BUILD_TEST_CHANNEL:-}" ]]; then
  bash scripts/check-release-hook-names.sh "$target/release/plexi"
fi
installer="$(bash scripts/distribution-tool.sh prepare)"
stage="$(mktemp -d "${TMPDIR:-/tmp}/plexi-source-package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
if [[ "$os" == macos && -z "${PLEXI_SIGN_IDENTITY:-}" ]] && security find-identity -v -p codesigning 2>/dev/null | grep -q 'Plexi Dev'; then
  export PLEXI_SIGN_IDENTITY='Plexi Dev'
fi
uv run --no-project --python 3.11 python scripts/package.py --binary "$target/release/plexi" --installer "$installer" --platform "$os-$arch" --channel "$channel" --output "$stage"
normalized="$channel"; [[ "$normalized" != main ]] || normalized=stable
"$installer" --package "$stage/package-$os-$arch-$normalized" --channel "$normalized" --install-only "$@"

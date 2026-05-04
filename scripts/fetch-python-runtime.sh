#!/usr/bin/env bash
# Usage: PYTHON_VERSION=x.y.z PYTHON_PBS_DATE=YYYYMMDD scripts/fetch-python-runtime.sh
# Downloads the python-build-standalone runtime into assets/python/ for bundling.
# Skips if the correct version is already present. macOS only.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "fetch-python-runtime: macOS only, skipping"
    exit 0
fi

ARCH=$(uname -m)
if [[ "$ARCH" == "arm64" ]]; then
    PBS_ARCH="aarch64-apple-darwin"
else
    PBS_ARCH="x86_64-apple-darwin"
fi

VERSION="${PYTHON_VERSION:?PYTHON_VERSION must be set}"
DATE="${PYTHON_PBS_DATE:?PYTHON_PBS_DATE must be set}"
EXPECTED="${VERSION}+${DATE}-${PBS_ARCH}"
VERSION_FILE="assets/python/.pbs-version"

if [[ -f "$VERSION_FILE" ]] && [[ "$(cat "$VERSION_FILE")" == "$EXPECTED" ]]; then
    echo "Python runtime ${VERSION} (${PBS_ARCH}) already present, skipping download"
    exit 0
fi

FILENAME="cpython-${VERSION}+${DATE}-${PBS_ARCH}-install_only.tar.gz"
URL="https://github.com/astral-sh/python-build-standalone/releases/download/${DATE}/${FILENAME}"

echo "Downloading Python ${VERSION} (${PBS_ARCH}) from python-build-standalone..."
rm -rf assets/python
mkdir -p assets

TMP=$(mktemp -d)
trap "rm -rf $TMP" EXIT

curl -fL --progress-bar "$URL" -o "$TMP/$FILENAME"
tar xzf "$TMP/$FILENAME" -C assets/

# Strip headers — not needed at runtime, saves ~5 MB
rm -rf assets/python/include

echo "$EXPECTED" > "$VERSION_FILE"
echo "Python ${VERSION} ready at assets/python/"

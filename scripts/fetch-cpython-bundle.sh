#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export LANG=C

VERSION="3.12.12"
WASI_SDK="20"
ARCHIVE="python-${VERSION}-wasi_sdk-${WASI_SDK}.zip"
ARCHIVE_SHA256="e40dac3ae68c988b9dcbf2ff6a1fb1b84435aa05b20defcd155801339f35feb2"
PYTHON_WASM_SHA256="62392f07fee032c22e3aa84be033c07105cd42424e5149058b9f5449a8deb272"
URL="https://github.com/brettcannon/cpython-wasi-build/releases/download/v${VERSION}/${ARCHIVE}"

CACHE_ROOT="${PLEXI_CPYTHON_BUNDLE_DIR:-${PLEXI_CONFIG_DIR:-$HOME/.plexi}/wasm-bundles}"
BUNDLE_DIR="${CACHE_ROOT}/cpython-${VERSION}"
PYTHON_WASM="${BUNDLE_DIR}/python.wasm"
VERSION_FILE="${BUNDLE_DIR}/.plexi-cpython-version"

sha256_file() {
    shasum -a 256 "$1" | awk '{print $1}'
}

if [[ -f "$PYTHON_WASM" ]] && [[ -f "$VERSION_FILE" ]]; then
    if [[ "$(cat "$VERSION_FILE")" == "${VERSION}+wasi_sdk-${WASI_SDK}" ]]; then
        actual="$(sha256_file "$PYTHON_WASM")"
        if [[ "$actual" == "$PYTHON_WASM_SHA256" ]]; then
            echo "CPython WASI ${VERSION} already present at ${BUNDLE_DIR}"
            exit 0
        fi
    fi
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading CPython WASI ${VERSION} from ${URL}"
curl -fL --progress-bar "$URL" -o "${tmp}/${ARCHIVE}"

archive_actual="$(sha256_file "${tmp}/${ARCHIVE}")"
if [[ "$archive_actual" != "$ARCHIVE_SHA256" ]]; then
    echo "error: archive SHA256 mismatch for ${ARCHIVE}" >&2
    echo "  expected: ${ARCHIVE_SHA256}" >&2
    echo "  actual:   ${archive_actual}" >&2
    exit 1
fi

mkdir -p "$BUNDLE_DIR"
rm -rf "${tmp}/extract"
mkdir -p "${tmp}/extract"
unzip -q "${tmp}/${ARCHIVE}" -d "${tmp}/extract"

if [[ ! -f "${tmp}/extract/python.wasm" ]]; then
    echo "error: ${ARCHIVE} did not contain python.wasm" >&2
    exit 1
fi

wasm_actual="$(sha256_file "${tmp}/extract/python.wasm")"
if [[ "$wasm_actual" != "$PYTHON_WASM_SHA256" ]]; then
    echo "error: python.wasm SHA256 mismatch" >&2
    echo "  expected: ${PYTHON_WASM_SHA256}" >&2
    echo "  actual:   ${wasm_actual}" >&2
    exit 1
fi

rm -rf "${BUNDLE_DIR:?}/"*
cp -R "${tmp}/extract/." "$BUNDLE_DIR/"
echo "${VERSION}+wasi_sdk-${WASI_SDK}" > "$VERSION_FILE"

echo "CPython WASI ${VERSION} ready at ${BUNDLE_DIR}"

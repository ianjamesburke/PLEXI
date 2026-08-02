#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export LANG=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="${ROOT}/apps/wasm-poc/python-shim"
WASM_OUT="${ROOT}/target/wasm32-wasip1/release/python_shim.wasm"
FIXTURE_OUT="${ROOT}/tests/wasm-fixtures/python-shim.wasm"

cd "$CRATE_DIR"
bash "$ROOT/scripts/cargo-with-lease.sh" cargo test
bash "$ROOT/scripts/cargo-with-lease.sh" cargo component build --release --target wasm32-wasip2
wasm-tools validate "$WASM_OUT"

mkdir -p "$(dirname "$FIXTURE_OUT")"
cp "$WASM_OUT" "$FIXTURE_OUT"
echo "Python shim fixture ready at ${FIXTURE_OUT}"

if [[ -n "${PLEXI_CPYTHON_BUNDLE_DIR:-}" ]]; then
    CACHE_OUT="${PLEXI_CPYTHON_BUNDLE_DIR}/cpython-3.12.12/plexi-python-shim.wasm"
    mkdir -p "$(dirname "$CACHE_OUT")"
    cp "$WASM_OUT" "$CACHE_OUT"
    echo "Python shim cache component ready at ${CACHE_OUT}"
fi

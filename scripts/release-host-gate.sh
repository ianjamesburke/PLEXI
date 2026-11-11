#!/usr/bin/env bash
set -euo pipefail
export RUSTFLAGS="-D warnings"
export PLEXI_CARGO_LEASE_TIMEOUT_SECS="${PLEXI_CARGO_LEASE_TIMEOUT_SECS:-86400}"
bash scripts/cargo-with-lease.sh cargo test --all-targets --no-run < /dev/null
bash scripts/cargo-with-lease.sh cargo test --bin plexi < /dev/null
bash scripts/cargo-with-lease.sh cargo clippy --bin plexi -- -D warnings < /dev/null

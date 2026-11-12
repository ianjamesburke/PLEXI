#!/usr/bin/env bash
set -euo pipefail
export RUSTFLAGS="-D warnings"
export PLEXI_CARGO_LEASE_TIMEOUT_SECS="${PLEXI_CARGO_LEASE_TIMEOUT_SECS:-86400}"
# Release CI builds this binary before the host suite; the SIGPIPE test runs it.
bash scripts/cargo-with-lease.sh cargo build --release --bin plexi < /dev/null
bash scripts/cargo-with-lease.sh cargo test --all-targets --no-run < /dev/null
bash scripts/cargo-with-lease.sh cargo test --bin plexi < /dev/null
bash scripts/cargo-with-lease.sh cargo clippy --bin plexi -- -D warnings < /dev/null

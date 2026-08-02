#!/usr/bin/env bash
# Run one Cargo leaf under the host's cargo-build lease when a Plexi host is
# reachable. CI and standalone shells have no host/lane concurrency, so they
# proceed explicitly unleased; a reachable host that times out is a hard fail.
set -euo pipefail

lease_name="cargo-build"
if [[ -z "${PLEXI_SOCKET:-}" ]]; then
  echo "cargo-lease: proceeding unleased (no Plexi host reachable)" >&2
  exec "$@"
fi

# `lock` is intentionally a new command, so first prove that the host itself
# is reachable through an established verb. A pre-lease shim must not turn an
# unknown `lock` command into an unleased build.
set +e
plexi pane list >/dev/null 2>&1
probe_rc=$?
set -e
if [[ "$probe_rc" -ne 0 ]]; then
  echo "cargo-lease: proceeding unleased (no Plexi host reachable)" >&2
  exec "$@"
fi

set +e
plexi lock status "$lease_name" >/dev/null 2>&1
status_rc=$?
set -e
if [[ "$status_rc" -ne 0 ]]; then
  echo "cargo-lease: host reachable but status failed (exit $status_rc); refusing unleased cargo" >&2
  exit "$status_rc"
fi

set +e
plexi lock acquire "$lease_name" --timeout "${PLEXI_CARGO_LEASE_TIMEOUT_SECS:-900}"
rc=$?
set -e
if [[ "$rc" -ne 0 ]]; then
  echo "cargo-lease: host reachable but acquire failed (exit $rc); refusing unleased cargo" >&2
  exit "$rc"
fi

release() {
  if ! plexi lock release "$lease_name" >/dev/null; then
    echo "cargo-lease: host reachable but release failed; refusing to hide a held lease" >&2
    return 1
  fi
}

cleanup() {
  local command_rc=$?
  trap - EXIT INT TERM
  release || exit 1
  exit "$command_rc"
}
trap cleanup EXIT INT TERM
"$@"

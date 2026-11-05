#!/usr/bin/env bash
# Verify the live host against the selected installed CLI and executable.
set -euo pipefail
if [[ $# != 2 ]]; then
    echo "usage: $0 <channel-cli> <expected-running-executable>" >&2
    exit 2
fi
cli=$1
expected=$2
status=$("$cli" host status --json)
build=$("$cli" --build-info)
python3 -c '
import json, os, sys
try:
    status, build = json.loads(sys.argv[1]), json.loads(sys.argv[2])
    running = status.get("running") or {}
    installed = (status.get("installed") or {}).get("active") or {}
    required = ("version", "build_id", "source_commit", "executable_path")
    if not status.get("ready") or status.get("running_identity") != "verified":
        raise ValueError("host identity is unverified or host is not ready")
    if any(not isinstance(running.get(k), str) or running[k] in ("", "unknown") for k in required):
        raise ValueError("running identity is incomplete")
    if not isinstance(running.get("pid"), int) or isinstance(running["pid"], bool) or running["pid"] <= 0:
        raise ValueError("running PID is unavailable")
    if os.path.realpath(running["executable_path"]) != os.path.realpath(sys.argv[3]):
        raise ValueError("running executable path differs from expected")
    if running["build_id"] != build.get("build_id") or running["source_commit"] != build.get("source_commit") or running["build_id"] != installed.get("build_id"):
        raise ValueError("running, CLI and installed build identities differ")
except (ValueError, TypeError, json.JSONDecodeError) as error:
    print(f"running build verification failed: {error}", file=sys.stderr)
    sys.exit(1)
print("running build verified")
' "$status" "$build" "$expected"

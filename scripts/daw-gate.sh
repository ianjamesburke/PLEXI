#!/usr/bin/env bash
# Installed-host DAW release gate (stint 0519). The DAW analog of
# `scripts/editor-gate.sh`, reusing the same named cases across every tier.
#
# Flow:
#   1. Core qualification (matrix + long sequence + seeded fuzz over the pure
#      model) — writes the daw-gate-core.json artifact; the gate fails outright
#      without it.
#   2. Boot ONE hermetic host for the channel (`host start --ephemeral`: no
#      workspace restore, no workspace save) and leave an info-level start
#      marker in the channel's plexi.log via `plexi host log`.
#   3. Run every DAW scene (tests/scenes/daw-*.toml) against that host in
#      attach mode (PLEXI_SCENE_ATTACH=1), collecting SceneReport JSONs and
#      failure bundles per scene, plus a best-effort bounded host screenshot
#      per scene (`plexi host screenshot` can stall — a hang or failure only
#      logs a warning and never affects the gate result).
#   4. Leave a finish marker with pass/fail counts in the channel log, stop
#      the host, tail the channel's plexi.log, and write summary.json.
#
# Exits nonzero when the core qualification, the host boot, or any scene
# fails. Screenshots and log markers are evidence, not gate conditions.
#
# Usage: daw-gate.sh <channel> [out_dir]
set -uo pipefail

channel="${1:?usage: daw-gate.sh <channel> [out_dir]}"
out_dir="${2:-/tmp/plexi-daw-gate/$channel}"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
scene_dir="$repo_root/tests/scenes"

if [[ "$channel" == "main" ]]; then
    plexi_bin="plexi"
    profile_dir="$HOME/.plexi"
else
    plexi_bin="plexi-$channel"
    profile_dir="$HOME/.plexi-$channel"
fi

mkdir -p "$out_dir"

# Run a command with a hard wall-clock bound (portable: macOS ships no
# `timeout`). Returns 124 on timeout, the command's exit code otherwise.
run_bounded() {
    local seconds="$1"
    shift
    "$@" &
    local pid=$!
    (
        sleep "$seconds"
        kill "$pid" 2>/dev/null
    ) &
    local watchdog=$!
    local code=0
    wait "$pid" 2>/dev/null || code=$?
    kill "$watchdog" 2>/dev/null
    wait "$watchdog" 2>/dev/null
    if [[ $code -ge 128 ]]; then
        return 124
    fi
    return "$code"
}

# Screenshot collection is evidence, not a release condition. Under fleet load
# the host can miss one client deadline while still being healthy, so retry a
# bounded request with backoff and leave a load diagnostic for the tester.
capture_screenshot_with_retry() {
    local output="$1"
    local log_file="$2"
    local attempt
    for attempt in 1 2 3; do
        if run_bounded 20 "$plexi_bin" host screenshot --output "$output" >> "$log_file" 2>&1; then
            return 0
        fi
        {
            echo "daw-gate: screenshot attempt $attempt/3 failed; machine load diagnostic:"
            uptime || true
            sysctl -n vm.loadavg 2>/dev/null || true
        } >> "$log_file"
        [[ $attempt -lt 3 ]] && sleep "$attempt"
    done
    return 1
}

# Host log marker: the installed run's start/finish summary must reach the
# channel's plexi.log, whose logger the host process owns. Bounded and
# warn-only: a marker failure never changes the gate result.
host_marker() {
    if ! run_bounded 10 "$plexi_bin" host log --source daw_gate "$1" \
        >> "$out_dir/host-markers.log" 2>&1; then
        echo "daw-gate: warning: host log marker failed: $1" >&2
    fi
}

gate_host_started=""
cleanup_gate_host() {
    if [[ -n "$gate_host_started" ]]; then
        "$plexi_bin" host stop >/dev/null 2>&1 || true
        gate_host_started=""
    fi
}
trap cleanup_gate_host EXIT INT TERM HUP

# Attach-eligible scene set. Only scenes that can run against a live installed
# host belong here: raw-WASM review is pre-approved by the live runner
# (`plexi app trust`), and the file picker is scripted through the host's
# PLEXI_PICKER_SCRIPT (set below) — never an in-scene `picker_script`, which is
# headless-only. daw-timeline drives transport/edit via named keys;
# daw-gate-bundle is the attach counterpart of the headless daw-bundle scene.
attach_scenes=(daw-timeline daw-gate-bundle)
scenes=()
for name in "${attach_scenes[@]}"; do
    scene="$scene_dir/$name.toml"
    if [[ ! -f "$scene" ]]; then
        echo "daw-gate: missing attach scene $scene" >&2
        exit 1
    fi
    # Category guard (stint 0519): an in-scene picker_script is honored only by
    # the in-process suite backend; the live runner rejects it. Such a scene is
    # not attach-eligible and must never silently enter the installed-host gate.
    if grep -Eq '^[[:space:]]*picker_script' "$scene"; then
        echo "daw-gate: scene $name declares picker_script (headless-only) and is not attach-eligible; script the picker via PLEXI_PICKER_SCRIPT at host start instead" >&2
        exit 1
    fi
    scenes+=("$scene")
done

echo "daw-gate: started channel=$channel scenes=${#scenes[@]} out=$out_dir"

# Core qualification first: run it fresh so the artifact always matches this
# working tree, and fail the gate outright when it fails.
core_artifact="${TMPDIR:-/tmp}/plexi-daw-gate/daw-gate-core.json"
rm -f "$core_artifact"
echo "daw-gate: running core qualification (cargo test daw_gate core artifact)"
if ! (cd "$repo_root" && bash scripts/cargo-with-lease.sh env -u PLEXI_CHANNEL -u PLEXI_CONTEXT_ROOT -u PLEXI_CONTEXT_ID \
    -u PLEXI_CONTEXT_NAME -u PLEXI_SOCKET -u PLEXI_RUNNING -u PLEXI_PANE_ID \
    cargo test --bin plexi daw_gate::daw_gate_core_qualification_artifact \
    > "$out_dir/core-qualification.log" 2>&1); then
    echo "daw-gate: CORE QUALIFICATION FAILED (see $out_dir/core-qualification.log)" >&2
    exit 1
fi
if [[ ! -f "$core_artifact" ]]; then
    echo "daw-gate: core qualification did not produce $core_artifact" >&2
    exit 1
fi
cp "$core_artifact" "$out_dir/daw-gate-core.json"
echo "daw-gate: collected core artifact daw-gate-core.json"

# Script the host's file picker for daw-gate-bundle's save-as / open / export
# (F2/F3/F5). The live host reads PLEXI_PICKER_SCRIPT per pane at launch, so it
# must be exported before `host start`; each pick grants write to the concrete
# path it returns. daw-timeline never opens a picker, so its unused queue is
# harmless.
picker_script="$out_dir/picker-script.json"
cat > "$picker_script" <<PICKER
[
  {"paths": ["$out_dir/song"]},
  {"paths": ["$out_dir/song"]},
  {"paths": ["$out_dir/mixdown.wav"]}
]
PICKER
export PLEXI_PICKER_SCRIPT="$picker_script"

# One hermetic host for the whole gate: --ephemeral means the channel's saved
# session is neither restored nor overwritten. The started flag is set before
# the attempt: a host that spawns but fails readiness still exists, and the
# EXIT trap must stop it rather than orphan it (stopping a never-started host
# is a bounded no-op).
gate_host_started=1
if ! "$plexi_bin" host start --ephemeral --pane "cwd=$repo_root" \
    > "$out_dir/host-start.log" 2>&1; then
    echo "daw-gate: HOST START FAILED (see $out_dir/host-start.log)" >&2
    exit 1
fi
host_marker "gate started channel=$channel scenes=${#scenes[@]}"

failures=0
scene_entries=""
for scene in "${scenes[@]}"; do
    name="$(basename "$scene" .toml)"
    scene_out="$out_dir/scenes/$name"
    mkdir -p "$scene_out"
    echo "daw-gate: running scene $name"
    if PLEXI_SCENE_ATTACH=1 bash "$repo_root/scripts/run-live-scene.sh" \
        "$scene" "$channel" "$scene_out" > "$scene_out/run.log" 2>&1; then
        passed=true
    else
        passed=false
        failures=$((failures + 1))
        echo "daw-gate: SCENE FAILED $name (see $scene_out/run.log)" >&2
    fi
    # Best-effort pixel evidence. `plexi host screenshot` can silently stall
    # (pre-existing host bug, tracked separately) — bound it hard and never
    # let it affect the gate result.
    if ! capture_screenshot_with_retry "$scene_out/host-after.png" "$scene_out/screenshot.log"; then
        echo "daw-gate: warning: screenshot after scene $name failed after retries; see load diagnostics (best-effort, gate unaffected)" \
            | tee -a "$scene_out/screenshot.log" >&2
    fi
    [[ -n "$scene_entries" ]] && scene_entries+=","
    scene_entries+=$'\n    '"{\"name\": \"$name\", \"passed\": $passed, \"report\": \"scenes/$name/$name.json\"}"
done

total=${#scenes[@]}
passed_count=$((total - failures))
host_marker "gate finished channel=$channel passed=$passed_count failed=$failures"
cleanup_gate_host

# Channel log tail for post-mortems (includes the markers above).
channel_log="$profile_dir/plexi.log"
if [[ -f "$channel_log" ]]; then
    tail -n 500 "$channel_log" > "$out_dir/log-tail.txt"
else
    echo "daw-gate: channel log $channel_log not found" > "$out_dir/log-tail.txt"
fi

cat > "$out_dir/summary.json" <<SUMMARY
{
  "channel": "$channel",
  "scenes": [$scene_entries
  ],
  "totals": {"scenes": $total, "passed": $passed_count, "failed": $failures}
}
SUMMARY

echo "daw-gate: finished channel=$channel passed=$passed_count failed=$failures summary=$out_dir/summary.json"
if [[ $failures -gt 0 ]]; then
    exit 1
fi

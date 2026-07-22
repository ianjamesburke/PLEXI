#!/usr/bin/env bash
# Installed-host editor release gate (stint 0479).
#
# Runs the core qualification (matrix + long sequence + seeded fuzz, which
# writes the editor-gate-core.json artifact), then every editor scene
# (tests/scenes/editor-gate-*.toml plus the notes-*.toml scenes) against one
# installed channel via run-live-scene.sh, collecting SceneReport JSONs and
# failure bundles into one out dir (live runs are semantic-only; pixel
# evidence comes from the headless scene suite's shot steps), tails the
# channel's plexi.log, and writes summary.json. Exits nonzero when the core
# qualification or any scene fails.
#
# Usage: editor-gate.sh <channel> [out_dir]
set -uo pipefail

channel="${1:?usage: editor-gate.sh <channel> [out_dir]}"
out_dir="${2:-/tmp/plexi-editor-gate/$channel}"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
scene_dir="$repo_root/tests/scenes"

mkdir -p "$out_dir"

scenes=()
for scene in "$scene_dir"/editor-gate-*.toml "$scene_dir"/notes-*.toml; do
    [[ -f "$scene" ]] && scenes+=("$scene")
done
if [[ ${#scenes[@]} -eq 0 ]]; then
    echo "editor-gate: no editor scenes found in $scene_dir" >&2
    exit 1
fi

echo "editor-gate: started channel=$channel scenes=${#scenes[@]} out=$out_dir"

# Core qualification first: run it fresh so the artifact always matches this
# working tree, and fail the gate outright when it fails.
core_artifact="${TMPDIR:-/tmp}/plexi-editor-gate/editor-gate-core.json"
rm -f "$core_artifact"
echo "editor-gate: running core qualification (cargo test editor::gate)"
if ! (cd "$repo_root" && env -u PLEXI_CHANNEL -u PLEXI_CONTEXT_ROOT -u PLEXI_CONTEXT_ID \
    -u PLEXI_CONTEXT_NAME -u PLEXI_SOCKET -u PLEXI_RUNNING -u PLEXI_PANE_ID \
    cargo test --bin plexi editor::gate::gate_core_qualification_artifact \
    > "$out_dir/core-qualification.log" 2>&1); then
    echo "editor-gate: CORE QUALIFICATION FAILED (see $out_dir/core-qualification.log)" >&2
    exit 1
fi
if [[ ! -f "$core_artifact" ]]; then
    echo "editor-gate: core qualification did not produce $core_artifact" >&2
    exit 1
fi
cp "$core_artifact" "$out_dir/editor-gate-core.json"
echo "editor-gate: collected core artifact editor-gate-core.json"

failures=0
scene_entries=""
for scene in "${scenes[@]}"; do
    name="$(basename "$scene" .toml)"
    scene_out="$out_dir/scenes/$name"
    mkdir -p "$scene_out"
    echo "editor-gate: running scene $name"
    if bash "$repo_root/scripts/run-live-scene.sh" "$scene" "$channel" "$scene_out" \
        > "$scene_out/run.log" 2>&1; then
        passed=true
    else
        passed=false
        failures=$((failures + 1))
        echo "editor-gate: SCENE FAILED $name (see $scene_out/run.log)" >&2
    fi
    [[ -n "$scene_entries" ]] && scene_entries+=","
    scene_entries+=$'\n    '"{\"name\": \"$name\", \"passed\": $passed, \"report\": \"scenes/$name/$name.json\"}"
done

# Channel log tail for post-mortems.
channel_log="$HOME/.plexi-$channel/plexi.log"
if [[ -f "$channel_log" ]]; then
    tail -n 500 "$channel_log" > "$out_dir/log-tail.txt"
else
    echo "editor-gate: channel log $channel_log not found" > "$out_dir/log-tail.txt"
fi

total=${#scenes[@]}
passed_count=$((total - failures))
cat > "$out_dir/summary.json" <<SUMMARY
{
  "channel": "$channel",
  "scenes": [$scene_entries
  ],
  "totals": {"scenes": $total, "passed": $passed_count, "failed": $failures}
}
SUMMARY

echo "editor-gate: finished channel=$channel passed=$passed_count failed=$failures summary=$out_dir/summary.json"
if [[ $failures -gt 0 ]]; then
    exit 1
fi

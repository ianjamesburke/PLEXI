# Prefer minimal recipes. Any recipe with real logic belongs in scripts/ and
# should be invoked here with a single `bash scripts/<name>.sh` call.

PYTHON_VERSION := "3.12.13"
PYTHON_PBS_DATE := "20260414"

export RUSTFLAGS := "-D warnings"

# Download the python-build-standalone runtime into assets/python/ for bundling.
# Skips if the correct version is already present. macOS only.
fetch-python-runtime:
    PYTHON_VERSION={{PYTHON_VERSION}} PYTHON_PBS_DATE={{PYTHON_PBS_DATE}} bash scripts/fetch-python-runtime.sh

dev:
    cargo run

# Run the full test suite — HostHarness regression tests + unit tests.
test:
    cargo test

build:
    cargo build --release

# Regenerate the canonical PGAP JSON Schema and Python protocol models.
# Run after any change to src/app_protocol.rs.
gen-schema:
    cargo run -p gen_schema > sdk/protocol/pgap.schema.json
    python3 tools/gen_protocol_py.py
    @echo "Schema and Python protocol models regenerated."

# Verify the committed schema is up to date with the current Rust source.
# Fails if sdk/protocol/pgap.schema.json is stale.
check-schema:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p gen_schema > /tmp/pgap_check.schema.json
    if ! diff -q sdk/protocol/pgap.schema.json /tmp/pgap_check.schema.json > /dev/null; then
        echo "ERROR: sdk/protocol/pgap.schema.json is stale. Run 'just gen-schema'."
        diff sdk/protocol/pgap.schema.json /tmp/pgap_check.schema.json || true
        exit 1
    fi
    echo "Schema is up to date."

run:
    cargo run --release

# Reads .channel from CWD and installs the appropriate build.
# Run from any worktree: just install
install: fetch-python-runtime
    bash scripts/install.sh

# Build and install the current worktree as a testable PR build.
# Installs as "Plexi PR<number>.app" with isolated profile ~/.plexi-pr-<number>/.
# Run from inside the feature worktree: just pr-install 123
pr-install number: fetch-python-runtime
    bash scripts/install.sh "pr-{{number}}"

# Remove a PR build: app bundle, CLI binary, and profile directory.
# Run after the PR is merged and approved: just pr-clean 123
pr-clean number:
    bash scripts/pr-clean.sh {{number}}

# Remove all PR builds whose GitHub PR is no longer open.
# Cleans app bundle, CLI binary, and profile directory for each closed/merged PR.
pr-clean-merged:
    bash scripts/pr-clean-merged.sh

# Wipe a channel's installed apps directory then re-sync from examples/.
# Useful when an app is renamed or removed — rsync won't delete stale dirs.
#   just clear-apps alpha
#   just clear-apps beta
#   just clear-apps stable
clear-apps channel="":
    bash scripts/clear-apps.sh {{channel}}

# Bump version, generate CHANGELOG via git-cliff, and commit. Defaults to patch.
# Run after merging a PR to alpha, before promoting to beta.
#   just bump           — patch bump
#   just bump minor     — minor bump
#   just bump major     — major bump
bump bump="patch":
    bash scripts/release-version.sh "{{bump}}"

# Promote to the next channel. Auto-detects current branch and prompts.
# Run just bump before promoting if you haven't already.
#   just promote        — detects alpha→beta or beta→main and confirms
#   just promote beta   — skip prompt, promote alpha→beta
#   just promote main   — skip prompt, promote beta→main
promote to="":
    bash scripts/promote.sh "{{to}}"

# Print the numbers of the top N unblocked open issues (default 1), labelling each "in progress".
# Enables: just next-issue 4 | while read -r num; do plexi open terminal --layout split_v "c /ship $num"; done
next-issue n="1":
    #!/usr/bin/env bash
    set -euo pipefail
    gh issue list --state open --limit 100 \
      --json number,labels \
      --jq '[.[] | select(.labels | map(.name) | (contains(["in progress"]) | not))] |
            sort_by(.labels | map(.name) |
              if contains(["P0"]) then 0
              elif contains(["P1"]) then 1
              elif contains(["P2"]) then 2
              elif contains(["P3"]) then 3
              else 4 end) |
            .[: {{n}}][].number' | \
    while read -r num; do
        gh issue edit "$num" --add-label "in progress" > /dev/null
        echo "$num"
    done

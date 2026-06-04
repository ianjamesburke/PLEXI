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

# Run the website dev server at http://localhost:4321
web:
    npm --prefix website run dev

# Run the full test suite — HostHarness regression tests + unit tests.
test:
    cargo test

# Generate an HTML line-coverage report and open it in the browser.
# Requires: cargo install cargo-llvm-cov && rustup component add llvm-tools-preview
coverage:
    cargo llvm-cov --bin plexi --html --open -- --skip "welcome_tab_falls_back_to_home_dir_when_no_root"

# Print per-file coverage summary to stdout (no browser).
coverage-summary:
    cargo llvm-cov --bin plexi --summary-only -- --skip "welcome_tab_falls_back_to_home_dir_when_no_root"

build:
    cargo build --release

# Regenerate the canonical PGAP JSON Schema and Python protocol models.
# Run after any change to src/app_protocol.rs.
gen-schema:
    cargo run -p gen_schema > sdk/protocol/pgap.schema.json
    python3 tools/gen_protocol_py.py
    @echo "Schema and Python protocol models regenerated."

# Regenerate the CLI reference docs from the clap Command tree.
# Run after any change to src/cli_args.rs.
gen-cli-docs:
    cargo run -p gen_cli_docs > website/src/content/docs/cli.md
    @echo "CLI reference regenerated."
    git add website/src/content/docs/cli.md
    git diff --cached --quiet || git commit -m "chore(website): regenerate CLI reference docs"
    git push

# Verify the committed CLI docs are up to date with the current Rust source.
# Fails if website/src/content/docs/cli.md is stale.
check-cli-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p gen_cli_docs > /tmp/plexi_check_cli.md
    if ! diff -q website/src/content/docs/cli.md /tmp/plexi_check_cli.md > /dev/null; then
        echo "ERROR: website/src/content/docs/cli.md is stale. Run 'just gen-cli-docs'."
        diff website/src/content/docs/cli.md /tmp/plexi_check_cli.md || true
        exit 1
    fi
    echo "CLI docs are up to date."

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

# Regenerate generated artifacts only when their source files changed.
# When adding a new generated artifact, add a stale check here.
#
# Source file                → Generated artifact(s)               → Generator
# src/cli_args.rs            → website/src/content/docs/cli.md     → cargo run -p gen_cli_docs
# src/app_protocol.rs        → sdk/protocol/pgap.schema.json       → cargo run -p gen_schema
#                            → sdk/python/plexi_sdk/_protocol.py   → python3 tools/gen_protocol_py.py
regen-if-stale:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ src/cli_args.rs -nt website/src/content/docs/cli.md ]]; then
        echo "cli_args.rs changed — regenerating CLI docs..."
        cargo run -p gen_cli_docs > website/src/content/docs/cli.md
    fi
    if [[ src/app_protocol.rs -nt sdk/protocol/pgap.schema.json ]]; then
        echo "app_protocol.rs changed — regenerating schema..."
        cargo run -p gen_schema > sdk/protocol/pgap.schema.json
        python3 tools/gen_protocol_py.py
    fi

# Derives channel from git branch (main/alpha/beta). Alias for: just channel-install
# Run from repo root or any worktree: just install
install: fetch-python-runtime regen-if-stale
    bash scripts/install.sh

# Editable install of plexi-sdk into your virtual environment for local development.
# Makes `plexi_sdk` importable in your IDE/type-checker with live source changes.
# Does not affect the SDK bundled into the Plexi app (use `just install` for that).
sdk-dev:
    uv pip install -e sdk/python

# Build and install the current worktree as a testable PR build.
# Installs as "Plexi PR<number>.app" with isolated profile ~/.plexi-pr-<number>/.
# Run from inside the feature worktree: just pr-install 123
# Always cleans the previous PR build first for a fully fresh install.
# Alias for: just channel-install pr-<number>
pr-install number: fetch-python-runtime
    bash scripts/pr-clean.sh {{number}}
    bash scripts/install.sh "pr-{{number}}"

# Remove a PR build: app bundle, CLI binary, and profile directory.
# Run after the PR is merged and approved: just pr-clean 123
# Alias for: just channel-clean pr-<number>
pr-clean number:
    bash scripts/pr-clean.sh {{number}}

# Remove all PR builds whose GitHub PR is no longer open.
# Cleans app bundle, CLI binary, and profile directory for each closed/merged PR.
pr-clean-merged:
    bash scripts/pr-clean-merged.sh

# Generic channel install. Auto-detects from branch if no channel given.
# Errors if branch is not main/alpha/beta — pass channel name explicitly for dev builds.
#   just channel-install          — auto-detect from branch
#   just channel-install gpui     — install as plexi-gpui channel
#   just channel-install pr-123   — install as PR build
channel-install channel="": fetch-python-runtime regen-if-stale
    bash scripts/install.sh {{channel}}

# Remove a channel: app bundle, CLI binary, and profile directory.
# Works for any channel name: main, alpha, beta, pr-123, gpui, etc.
#   just channel-clean gpui       — remove plexi-gpui artifacts
#   just channel-clean pr-123     — remove PR 123 artifacts
channel-clean channel:
    bash scripts/channel-clean.sh {{channel}}

# Remove all ephemeral channels (pr-*) whose GitHub PR is no longer open.
channel-clean-merged:
    bash scripts/channel-clean-merged.sh

# List all installed Plexi channels with tier, binary path, and profile dir.
channel-list:
    bash scripts/channel-list.sh

# Remove ALL Plexi channels plus shell integration and completions.
# Same as: just uninstall all
channel-uninstall channel="all":
    bash scripts/uninstall.sh {{channel}}

# Wipe a channel's installed apps directory then re-sync from examples/.
# Useful when an app is renamed or removed — rsync won't delete stale dirs.
#   just clear-apps alpha
#   just clear-apps beta
#   just clear-apps main
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

# Remove a Plexi channel and its profile dir, app bundle, and CLI binary.
# Defaults to removing all channels plus shell integration and completions.
# Backlog folders inside profile dirs are archived to ~/plexi-backlog-archive/.
#   just uninstall              — remove all channels
#   just uninstall main         — remove main only (also removes bare symlink, shell integration, completions)
#   just uninstall alpha        — remove alpha only
#   just uninstall beta         — remove beta only
#   just uninstall pr-123       — remove a specific PR build
#   just uninstall gpui         — remove any named development channel
uninstall channel="all":
    bash scripts/uninstall.sh {{channel}}

# Dispatch Claude agents at one or more issues. Labels each "in progress" first
# to prevent double-claiming when multiple dispatches run close together.
# TODO: replace `c` with `claude --dangerously-skip-permissions` once this flow
# is validated — if this comment still exists, that replacement never happened.
ship +issues:
    for issue in {{issues}}; do \
        gh issue edit "$issue" --add-label "in progress" --remove-label "ready" && \
        plexi terminal "c '/ship-issue $issue'" --layout new_window; \
    done

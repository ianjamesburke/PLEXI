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

# Rebuild the WASM POC components and refresh the committed test fixtures the
# host runtime gate tests load (G3/G5 etc). Run after changing any
# apps/wasm-poc/* source. Requires cargo-component + the wasm32-wasip2 target.
# Note: cargo-component emits the adapted component under the wasip1 path.
wasm-fixtures:
    cd apps/wasm-poc/sysmon && cargo component build --release --target wasm32-wasip2
    cp target/wasm32-wasip1/release/sysmon.wasm tests/wasm-fixtures/sysmon.wasm
    cd apps/wasm-poc/audio-synth && cargo component build --release --target wasm32-wasip2
    cp target/wasm32-wasip1/release/audio_synth.wasm tests/wasm-fixtures/audio-synth.wasm
    cd apps/wasm-poc/pong && cargo component build --release --target wasm32-wasip2
    cp target/wasm32-wasip1/release/pong.wasm tests/wasm-fixtures/pong.wasm
    cd apps/wasm-poc/breakout && cargo component build --release --target wasm32-wasip2
    cp target/wasm32-wasip1/release/breakout.wasm tests/wasm-fixtures/breakout.wasm
    cd apps/wasm-poc/counter && cargo component build --release --target wasm32-wasip2
    cp target/wasm32-wasip1/release/counter.wasm tests/wasm-fixtures/counter.wasm

# Generate an HTML line-coverage report and open it in the browser.
# Requires: cargo install cargo-llvm-cov && rustup component add llvm-tools-preview
coverage:
    cargo llvm-cov --bin plexi --html --open -- --skip "welcome_tab_falls_back_to_home_dir_when_no_root"

# Print per-file coverage summary to stdout (no browser).
coverage-summary:
    cargo llvm-cov --bin plexi --summary-only -- --skip "welcome_tab_falls_back_to_home_dir_when_no_root"

# Perf lint gate — run before and after any perf work; must exit clean.
# Scoped to the perf lint set only (`-A clippy::all` silences unrelated lints);
# vendored deps/egui_term opts out via crate-level allows in its lib.rs.
perf-clippy:
    RUSTC_WRAPPER= RUSTFLAGS= cargo clippy --all-targets --all-features --locked -- -A clippy::all -D clippy::perf -D clippy::redundant_clone -D clippy::needless_collect -D clippy::large_enum_variant -D clippy::clone_on_copy

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

# Regenerate the config reference docs from the serde config structs.
# Run after any change to src/config/mod.rs or scripts/default-config.toml.
gen-config-docs:
    cargo run -p gen_config_docs > website/src/content/docs/config.md
    @echo "Config reference regenerated."
    git add website/src/content/docs/config.md
    git diff --cached --quiet || git commit -m "chore(website): regenerate config reference docs"
    git push

# Verify the committed config docs are up to date with the current Rust source.
# Fails if website/src/content/docs/config.md is stale.
check-config-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p gen_config_docs > /tmp/plexi_check_config.md
    if ! diff -q website/src/content/docs/config.md /tmp/plexi_check_config.md > /dev/null; then
        echo "ERROR: website/src/content/docs/config.md is stale. Run 'just gen-config-docs'."
        diff website/src/content/docs/config.md /tmp/plexi_check_config.md || true
        exit 1
    fi
    echo "Config docs are up to date."

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

# Generate SDK reference docs from Python docstrings.
# Run after any change to sdk/python/plexi_sdk/.
gen-sdk-docs:
    python3 tools/gen_sdk_docs.py

# Verify the committed SDK docs are up to date with the current SDK source.
# Fails if website/src/content/docs/sdk.md is stale.
check-sdk-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 tools/gen_sdk_docs.py --stdout > /tmp/plexi_check_sdk.md
    if ! diff -q website/src/content/docs/sdk.md /tmp/plexi_check_sdk.md > /dev/null; then
        echo "ERROR: website/src/content/docs/sdk.md is stale. Run 'just gen-sdk-docs'."
        diff website/src/content/docs/sdk.md /tmp/plexi_check_sdk.md || true
        exit 1
    fi
    echo "SDK docs are up to date."

# Generate PGAP capability reference docs from the protocol JSON schema.
# Run after any change to src/app_protocol.rs (via just gen-schema first).
gen-capability-docs:
    python3 tools/gen_capability_docs.py

# Verify the committed PGAP docs are up to date with the current schema.
# Fails if website/src/content/docs/pgap.md is stale.
check-capability-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 tools/gen_capability_docs.py --stdout > /tmp/plexi_check_pgap.md
    if ! diff -q website/src/content/docs/pgap.md /tmp/plexi_check_pgap.md > /dev/null; then
        echo "ERROR: website/src/content/docs/pgap.md is stale. Run 'just gen-capability-docs'."
        diff website/src/content/docs/pgap.md /tmp/plexi_check_pgap.md || true
        exit 1
    fi
    echo "PGAP capability docs are up to date."

# Assert every non-hidden CLI command is mentioned in at least one docs page.
check-docs-coverage:
    bash tools/check_docs_coverage.sh

# Run all docs checks (CLI freshness + config freshness + SDK freshness + capability docs + command coverage).
check-docs: check-cli-docs check-config-docs check-sdk-docs check-capability-docs check-docs-coverage

run:
    cargo run --release

# Regenerate generated artifacts only when their source files changed.
# When adding a new generated artifact, add a stale check here.
#
# Source file                        → Generated artifact(s)                     → Generator / Checker
# Cargo.toml                         → website/src/content/docs/cli.md           → cargo run -p gen_cli_docs
# src/cli/args.rs                    → website/src/content/docs/cli.md           → cargo run -p gen_cli_docs
# src/config/mod.rs                  → website/src/content/docs/config.md        → cargo run -p gen_config_docs
# scripts/default-config.toml        → website/src/content/docs/config.md        → cargo run -p gen_config_docs
# src/app_protocol.rs                → sdk/protocol/pgap.schema.json             → cargo run -p gen_schema
#                                    → sdk/python/plexi_sdk/_protocol.py         → python3 tools/gen_protocol_py.py
# website/src/content/docs/          → (coverage assertion)                      → tools/check_docs_coverage.sh
regen-if-stale:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ Cargo.toml -nt website/src/content/docs/cli.md || src/cli/args.rs -nt website/src/content/docs/cli.md ]]; then
        echo "CLI docs source changed — regenerating CLI docs..."
        cargo run -p gen_cli_docs > website/src/content/docs/cli.md
    fi
    if [[ src/config/mod.rs -nt website/src/content/docs/config.md || scripts/default-config.toml -nt website/src/content/docs/config.md ]]; then
        echo "Config source changed — regenerating config docs..."
        cargo run -p gen_config_docs > website/src/content/docs/config.md
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

# Headless SDK/app contract smoke: Init -> Ready -> Render -> FrameDone.
# Pin to the bundled runtime's minor version so uv never falls back to a
# system Python (e.g. CommandLineTools 3.9) that predates the SDK's floor.
sdk-smoke:
    uv run --python 3.12 --project sdk/python --with pytest --with pytest-asyncio pytest sdk/python/tests/test_v3_adapter.py sdk/python/tests/test_v3_runtime_regression.py -q

# Build and install the current worktree as a testable PR build.
# Installs as "Plexi PR<number>.app" with isolated profile ~/.plexi-pr-<number>/.
# Run from inside the feature worktree: just pr-install 123
# Always cleans the previous PR build first for a fully fresh install.
# Alias for: just channel-install pr-<number>
pr-install number: fetch-python-runtime sdk-smoke
    #!/usr/bin/env bash
    set -euo pipefail
    # Preflight: confirm we're running from a valid repo worktree with Cargo.toml present.
    if [[ ! -f "Cargo.toml" ]] || ! grep -q 'name = "plexi"' Cargo.toml 2>/dev/null; then
      echo "Error: pr-install must be run from inside the PLEXI repo root or a worktree."
      echo "  Current directory: $(pwd)"
      echo "  Expected a Cargo.toml with name = \"plexi\"."
      exit 1
    fi
    # Preflight: confirm cargo metadata resolves a target-dir before starting a build.
    _target_dir="$(cargo metadata --format-version=1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "")"
    if [[ -z "$_target_dir" ]]; then
      echo "Error: could not resolve cargo target directory."
      echo "  Make sure 'cargo metadata' runs cleanly from $(pwd) and Python 3 is available."
      exit 1
    fi
    bash scripts/pr-clean.sh {{number}}
    bash scripts/install.sh "pr-{{number}}"

# Generic channel install. Auto-detects from branch if no channel given.
# Errors if branch is not main/alpha/beta — pass channel name explicitly for dev builds.
#   just channel-install          — auto-detect from branch
#   just channel-install gpui     — install as plexi-gpui channel
#   just channel-install pr-123   — install as PR build
#   just channel-install rc-010   — install a stable-tier local release candidate
channel-install channel="": fetch-python-runtime regen-if-stale
    bash scripts/install.sh {{channel}}

# Remove a channel: app bundle, CLI binary, and profile directory.
# Works for any channel name: main, alpha, beta, pr-123, gpui, etc.
#   just channel-clean gpui       — remove plexi-gpui artifacts
#   just channel-clean pr-123     — remove PR 123 artifacts
#   just channel-clean rc-010     — remove local release candidate artifacts
channel-clean channel:
    bash scripts/channel-clean.sh {{channel}}

# Remove all ephemeral channels (pr-*) whose GitHub PR is no longer open.
channel-clean-merged:
    bash scripts/channel-clean-merged.sh

# Remove target/ from worktrees whose branch is already merged to alpha.
# Safe: never touches active branches.
clean-stale-targets:
    bash scripts/clean-stale-targets.sh

# Remove whole worktrees whose PR is MERGED — only when clean and fully pushed.
# Destructive (deletes worktree + local branch); keeps open-PR, pre-PR, dirty,
# and unpushed worktrees. Pass `dry-run` to preview without deleting.
#   just reap-merged-worktrees           — remove safe merged worktrees
#   just reap-merged-worktrees dry-run   — preview verdicts only
reap-merged-worktrees mode="":
    bash scripts/reap-merged-worktrees.sh {{mode}}

# Run `cargo clean` in every worktree to reclaim incremental/debug artifacts.
# Deps cache is preserved (only removes target/debug and incremental/).
# Use when disk is low and you don't want to full-clean.
trim-targets:
    bash scripts/trim-targets.sh

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

# Preview what the next changelog would look like without committing anything.
changelog:
    git cliff --unreleased --bump

# Bump version, generate CHANGELOG via git-cliff, commit, and tag locally. Defaults to patch.
# Run at the end of a release batch, or immediately before promoting to beta.
#   just bump           — patch bump
#   just bump minor     — minor bump
#   just bump major     — major bump
bump bump="patch":
    bash scripts/release-version.sh "{{bump}}"

# Promote to the next channel. Auto-detects current branch and prompts.
# Run just bump before promoting if you haven't already.
#   just promote              — detects alpha→beta or beta→main and confirms
#   just promote beta         — skip prompt, promote alpha→beta
#   just promote main         — skip prompt, promote beta→main
#   just promote beta install — promote and install the target channel
#   just promote main install — promote and install the target channel
promote to="" install="":
    bash scripts/promote.sh "{{to}}" "{{install}}"

# Push the version tag for the current Cargo.toml version and trigger the GitHub Actions release.
# Run this only when you want a binary release — promote to main first.
release:
    bash scripts/release-tag.sh

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

# Squash-merge a validated PR to alpha: rebase, merge, sync, cleanup, close issue.
# Intended for the merge-pr skill — run from repo root.
#   just merge-pr 2155
#   just merge-pr 2202 no-issue   — standalone follow-up PR, skips issue/stint close
merge-pr PR *FLAGS:
    bash scripts/merge-pr.sh {{PR}} {{FLAGS}}

# Sub-steps for conflict recovery. Call individually to resume a failed merge-pr.
#   just merge-rebase feature/2155-foo   — rebase + force-push feature branch
#   just merge-squash 2155               — squash-merge only
#   just merge-sync                      — reset local alpha to origin/alpha
#   just merge-cleanup 2155 feature/2155-foo
#   just merge-close 2144 2155
#   just merge-close-stints 2186 0015 0016
merge-rebase BRANCH:
    bash scripts/merge-pr.sh rebase {{BRANCH}}

merge-squash PR:
    bash scripts/merge-pr.sh squash {{PR}}

merge-sync:
    bash scripts/merge-pr.sh sync

merge-cleanup PR BRANCH:
    bash scripts/merge-pr.sh cleanup {{PR}} {{BRANCH}}

merge-close ISSUE PR:
    bash scripts/merge-pr.sh close {{ISSUE}} {{PR}}

merge-close-stints PR +STINTS:
    bash scripts/merge-pr.sh close-stints {{PR}} {{STINTS}}

# Dispatch Claude agents at one or more issues. Labels each "in progress" first
# to prevent double-claiming when multiple dispatches run close together.
# TODO: replace `c` with `claude --dangerously-skip-permissions` once this flow
# is validated — if this comment still exists, that replacement never happened.
ship +issues:
    for issue in {{issues}}; do \
        gh issue edit "$issue" --add-label "in progress" --remove-label "ready" && \
        plexi pane new "c '/ship-issue $issue'" --window; \
    done

# Run one UI scene file headlessly. Writes screenshots + a SceneReport JSON to
# the out dir. shots="0" skips screenshot steps (state-only, faster).
scene FILE out="/tmp/plexi-scenes" shots="1":
    PLEXI_SCENE={{FILE}} PLEXI_SCENE_OUT={{out}} \
    {{ if shots == "0" { "PLEXI_SCENE_NO_SHOTS=1" } else { "" } }} \
    cargo test --bin plexi scene_single -- --ignored --exact scenes::tests::scene_single --nocapture

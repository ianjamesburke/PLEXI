# Channel binary for the V1 acceptance suite

The acceptance target is the alpha channel binary built from this tree. `scripts/v1-acceptance.sh` copies `target/release/plexi` to a real file named `plexi-alpha` and exports that path. It does not assemble an integration branch, and it does not drive a bare `plexi` (that binary adopts `PLEXI_CHANNEL`).

The harness exports two variables before it runs an item script:

- `PLEXI_BIN` — absolute path of `plexi-alpha` (or an explicit `--bin` / `--pr` override)
- `PLEXI_E2E_SHIM` — absolute path of `scripts/e2e/plexi-bin.sh`

A channel-named binary ignores `PLEXI_CHANNEL`. The profile directory is the binary's basename (`plexi-alpha` writes `~/.plexi-alpha`). Sourcing the shim is what makes that true for a script that still hardcodes `plexi-pr-<PR>`.

Apply the line on the PR that owns the script. The acceptance suite does not edit those files.

## Channel binary

Replace the two-line `PR` / `BIN` pair with the source line.

`scripts/e2e_agents_api_installed.sh` (#2706). Replace:

```bash
PR="${1:?usage: scripts/e2e_agents_api_installed.sh <PR>}"
BIN="plexi-pr-${PR}"
```

with:

```bash
source "${PLEXI_E2E_SHIM:?}"
```

`scripts/multi-lead-e2e.sh` (#2715). Replace:

```bash
PR="${1:?usage: scripts/multi-lead-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
```

with:

```bash
source "${PLEXI_E2E_SHIM:?}"
```

`scripts/headless-queue-e2e.sh` (#2716). Replace:

```bash
PR="${1:?usage: scripts/headless-queue-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
```

with:

```bash
source "${PLEXI_E2E_SHIM:?}"
```

`scripts/command-view-steer-e2e.sh` (#2717). Replace:

```bash
PR="${1:?usage: scripts/command-view-steer-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
```

with:

```bash
source "${PLEXI_E2E_SHIM:?}"
```

`scripts/cloud-basics-e2e.sh` (#2703). Replace:

```bash
PR="${1:?usage: scripts/cloud-basics-e2e.sh <PR>}"
BIN="plexi-pr-${PR}"
```

with:

```bash
source "${PLEXI_E2E_SHIM:?}"
```

`services/relay/e2e_installed.sh` (#2708) already reads `PLEXI_BIN`. It still runs `export PLEXI_CHANNEL="pr-${PR}"` with `PR` defaulting to `2685`. Replace that export with:

```bash
unset PLEXI_CHANNEL
```

`scripts/folder-secrets-e2e.sh` (#2705 / #2719) takes the binary as `$1`. The harness passes it. So a direct run uses the same variable, replace:

```bash
PLEXI="${1:-$REPO_ROOT/target/release/plexi}"
```

with:

```bash
PLEXI="${PLEXI_BIN:-${1:-$REPO_ROOT/target/release/plexi}}"
```

`scripts/e2e/ledger/run.sh` (#2710) already uses `PLEXI_BIN` and derives the profile from the basename. No edit when the harness exports `PLEXI_BIN`.

Until those PRs land, the harness runs a same-directory copy of each `<PR>`-only script with that source line already applied, then deletes the copy. The profile is still the channel binary's, not `~/.plexi-pr-<N>`.

## Human click

Done means the installed script passed and the approval was a real click (`HUMAN_APPROVE` in `scripts/e2e/human.sh`). A CLI resolve is not an approval. The harness fails an item that used one of these, and names which:

- `skip panes` — `PLEXI_E2E_SKIP_PANES=1` (the contract says to use it only where an item notes it; none do)
- `CLI approval` — a positive `assistant permission resolve`, `needs-you resolve --approve`, `permissions allow`, `secret grant`, `command-view resolve`, or `changes accept` that is not asserted to fail
- `env flag` — an e2e env var other than skip-panes that selects the non-human path

A negative (`must not grant`, `does not record an allow`, `permission_denied`) is not `CLI approval`. No contract item permits any of the three. A required click that the script never makes is `bypass: none` and is still a failure.

`scripts/permissions-seal-e2e.sh` (#2718), V1-04. Bypass: none. After the forged-grant checks, source `scripts/e2e/human.sh` and `HUMAN_APPROVE` the Permissions-app grant, then the revoke (contract step 4). Do not call `permissions allow`.

`scripts/needs-you-e2e.sh` (#2704), V1-05. Bypass: none. Keep `needs-you resolve --approve` as the assertion that a terminal resolve does not grant. After the id is listed, `HUMAN_APPROVE "$ID"` so the move commits (contract step 2).

`scripts/needs-you-persist-e2e.sh` (#2713), V1-05. Bypass: none. Keep both `needs-you resolve --approve` assertions as refusals. The click belongs in `needs-you-e2e.sh`.

`scripts/folder-secrets-e2e.sh` (#2705 / #2719), V1-06. Bypass: none. Keep `secret grant` as the assertion that it does not record an allow. Contract step 4 needs `HUMAN_APPROVE` for the KB grant. Do not set `PLEXI_E2E_SKIP_PANES`.

`services/relay/e2e_installed.sh` (#2708), V1-08. Bypass: none. Confirm the pairing code with `HUMAN_APPROVE` (contract step 1). An unconfirmed code must still yield no device.

`scripts/e2e_agents_api_installed.sh` (#2706), V1-09. Bypass: CLI approval. Replace:

```bash
RESOLVE="$(cli assistant permission resolve "$PENDING" --choice once)"
```

with `HUMAN_APPROVE "$PENDING"` after sourcing `scripts/e2e/human.sh`. Keep the assert that the review then succeeds.

`scripts/change-sets-e2e.sh` (#2724), V1-12. Bypass: CLI approval when `human.sh` is missing (the script prints `VERIFIED-VIA-BYPASS` and runs `assistant permission resolve` / `changes accept`). Delete that fallback. If `human.sh` is missing, exit non-zero.

`scripts/app-share-e2e.sh` (#2718), V1-15. Bypass: none. When the sensitive tool asks, `HUMAN_APPROVE` the pending id (contract step 4). Do not auto-grant it.

`scripts/multi-lead-e2e.sh` and `scripts/headless-queue-e2e.sh` (V1-10) and `scripts/command-view-steer-e2e.sh` (V1-11) echo `VERIFIED-VIA-BYPASS` because an ungranted tool must not run and a command-view resolve from an agent pane must be refused. That echo is not skip panes, CLI approval, or an env flag. Do not add a resolve to clear it.

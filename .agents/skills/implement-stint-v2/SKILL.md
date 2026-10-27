---
name: implement-stint-v2
description: "Implement one or more stints in an isolated feature worktree, validate with phase-appropriate pre-push gates, open a PR against alpha, and handle PASS / FOLLOW-UP / FAIL through bs-merge and channel-clean."
risk: low
source: local
---

# Implement Stint v2

Implements one or more `.stint` tasks into a single PR targeting `alpha`, coordinates state via Plexi pane slots, and cleanly retires the worktree and PR channel on merge.

When a babysitter brief explicitly selects `feature/alpha-cleanup` aggregate mode, the [aggregate instructions](../babysitter-v3/SKILL.md) override this skill's alpha-only start, PR base, and PASS merge steps. Use `just bs-start --base feature/alpha-cleanup <ids...>` and open the batch PR against that branch. **The head owns the merge: after PASS it runs `just bs-integrate` itself. You never run `integrate`, `just bs-integrate`, or `just bs-merge`, and you are not woken for a PASS.** Keep the stints claimed and do not mark them done before the combined PR reaches alpha. All implementation and pre-push gates below still apply.

```
/implement-stint-v2 <STINT_ID> [<STINT_ID> ...]
```

---

## Step 1 — Start the Lane (`just bs-start`)

Run all commands in the foreground. If your turn ends while a job runs in the background, nothing wakes you.

From the repo root (`alpha`), start the lane with `just bs-start`:

```bash
just bs-start <stint-id> [<stint-id> ...]
```

`just bs-start` verifies that `alpha` is clean and synced with `origin/alpha`, mints `worktrees/feature/stint-<id>[-<id>...]-<slug>` from `alpha` HEAD, and runs `stint claim` on every stint.

If `just bs-start` fails because `alpha` is dirty or out of sync:
```bash
plexi pane slot write last_error "alpha is dirty or out of sync" --pane-id $PLEXI_PANE_ID --replace
plexi pane slot write status impl:blocked --pane-id $PLEXI_PANE_ID --replace
plexi notify --title "Blocked: implement-stint-v2 <stint-id>" \
  --body "Stint <stint-id> could not start because alpha is dirty or out of sync." \
  --choice "Go to Context:pane_focus:$PLEXI_PANE_ID" \
  --scope context
```
Stop and wait for resolution.

Once `bs-start` succeeds, publish your working slot state and name the pane:
```bash
plexi pane slot write status impl:working --pane-id $PLEXI_PANE_ID --replace
plexi pane slot write pipeline_phase "implement-stint-v2:<stint-id>" --pane-id $PLEXI_PANE_ID --replace
plexi pane name $PLEXI_PANE_ID "impl-<stint-id>-<short-slug>"
```

`cd` into the worktree path printed by `just bs-start` (`worktrees/feature/stint-<id>[-<id>...]-<slug>`). **All implementation work lives inside this worktree.**

---

## Step 2 — Implement by Stint Size

Inspect the stint spec(s) first:
```bash
stint show <stint-id>
```
Read the `AGENTS.md` in any directory you plan to edit.

- **Size `S` (small):**
  - Locate the target code, write the failing test first (for host logic) or identify the screenshot harness surface (for host UI), implement the fix, and verify with targeted tests.
- **Size `M` (medium):**
  - Read the stint description and relevant code paths to produce a concrete execution plan before editing. If subagents are available, you may use a planning subagent to draft the plan and an implementation subagent to execute it, or execute the plan directly in-pane.
- **Size `L` (large):**
  - Break the work into coherent multi-phase slices.
  - For each phase: implement the slice → run focused tests and review code quality → fix any findings → checkpoint-commit on the worktree branch before starting the next phase.

---

## Step 3 — Mandatory Pre-Push Gate (`src/testing/TESTING.md`)

Before pushing, run every required gate in the foreground from your worktree:

1. **Focused tests** for the changed module(s).
2. **Release build (`-D warnings`):**
   ```bash
   just build
   ```
3. **Relevant scoped tests:** Follow [TESTING.md — The Gate](../../../../src/testing/TESTING.md#the-gate) for feature/batch selection, evidence, and independent installed checks. Run the selected libtest filters with its array invocation and stdin/environment isolation; include affected Python/shell/docs/SDK checks. Do not run a full host suite for a feature or batch integration. The final combined candidate uses the separate full-suite gate in TESTING.md. Record the actual scope and commands at the current head; hosted Rust checks stay deferred and other checks still gate.
4. **Clippy lint gate:**
   ```bash
   bash scripts/cargo-with-lease.sh cargo clippy --bin plexi -- -D warnings
   ```
5. **Docs/schema gate** (when CLI args, config fields, SDK, or docs changed):
   ```bash
   just check-docs
   ```
6. **Visual verification** (mandatory for host UI or PGAP app UI changes):
   - Render realistic seeded content via a `HostHarness` PNG test or `plexi app render <app> --png <path>`, inspect the PNG, and remove any temporary PNG before committing.

---

## Step 4 — Push, Open PR, and Publish `pr:done`

1. Commit cleanly (conventional commit message referencing `(stint <id>)`) and push the branch:
   ```bash
   git push -u origin <branch>
   ```
2. Open the PR against `alpha`. Always include `Stint: <id>` (or `Stints: <id>, <id>`) in the PR body so `just merge-pr` automatically closes the stint(s):
   ```bash
   gh pr create --base alpha --head "<branch>" --title "<type>(<scope>): <summary> (stint <id>)" --body "$(cat <<'EOF'
   ## Summary
   - <what changed and why>

   Stint: <id>

   ## Testing
   - <tests and gates run>
   EOF
   )"
   ```
3. Publish the PR number and `status=pr:done`:
   ```bash
   plexi pane slot write pr <PR_NUMBER> --pane-id $PLEXI_PANE_ID --replace
   plexi pane slot write status pr:done --pane-id $PLEXI_PANE_ID --replace
   ```
4. **Install rule:**
   - **When run by `babysitter-v3` (unattended):** Do **not** run `just pr-install` (the dedicated tester pane installs and drives the PR build).
   - **When run standalone by Ian (interactive):** Run `just pr-install <PR_NUMBER>` in the foreground so the PR build is ready for spot-checking.
5. Output the completion block and wait for `PASS`, `FOLLOW-UP: <list>`, or `FAIL: <reason>`:

```text
[IMPLEMENTED]
Stint: <task-id>
PR: #<PR_NUMBER>
Branch: <branch>
Files changed: <N>

TESTING:
- <commands to run or behavior to verify against the Done-When>

"Pass, follow up, or fail?"
```

---

## Step 5 — Handle the Verdict

### On `FOLLOW-UP: <items>`
1. Immediately flip your slot back to working before doing anything else:
   ```bash
   plexi pane slot write status impl:working --pane-id $PLEXI_PANE_ID --replace
   ```
2. Diagnose and fix the root cause (not a surface patch) inside the same worktree (`worktrees/feature/stint-...`).
3. Re-run the Step 3 pre-push gates, commit, and push to the same branch.
4. (Standalone interactive mode only: re-run `just pr-install <PR_NUMBER>`.)
5. Write `plexi pane slot write status pr:done --pane-id $PLEXI_PANE_ID --replace` and post the updated `[IMPLEMENTED]` block.

### On `PASS`
1. Flip your slot to `merge:working`:
   ```bash
   plexi pane slot write status merge:working --pane-id $PLEXI_PANE_ID --replace
   ```
2. Confirm the phase-appropriate local gate, independent installed affected-behavior validation and non-deferred PR checks are green for the head being merged (TESTING.md). Batch integration does not require a full suite; the final combined candidate does. Move to the `alpha` root checkout first (never remove a worktree while standing inside it):
   ```bash
   cd "$(git worktree list --porcelain | awk '/^worktree /{p=substr($0,10)} /^branch refs\/heads\/alpha$/{print p; exit}')"
   ```
3. Merge and retire the lane with `just bs-merge`:
   ```bash
   just bs-merge <PR_NUMBER>
   ```
   `just bs-merge` rebases and squash-merges the PR into `alpha`, syncs local `alpha`, marks all linked stints `done`, cleans up the feature worktree and remote branch, sweeps merged worktrees/stale targets, uninstalls `plexi-pr-<PR_NUMBER>` (`just channel-clean pr-<PR_NUMBER>`), and publishes `status=merge:done`.
4. Confirm `stint show <id>` shows `status: done` and write:
   ```bash
   plexi pane slot write status merge:done --pane-id $PLEXI_PANE_ID --replace
   ```
   (If running standalone without `babysitter-v3`, also update `TRUTH.md` at the repo root.)

### On `FAIL: <reason>`
1. Move to the `alpha` root checkout:
   ```bash
   cd "$(git worktree list --porcelain | awk '/^worktree /{p=substr($0,10)} /^branch refs\/heads\/alpha$/{print p; exit}')"
   ```
2. Add or update `## Prior Attempts` in `.stint/tasks/<id>-*.md` with what was tried, why it failed, and gotchas for the next attempt.
3. Record the failure on your own pane:
   ```bash
   plexi pane slot write last_error "<one-line failure summary>" --pane-id $PLEXI_PANE_ID --replace
   plexi pane slot write status impl:failed --pane-id $PLEXI_PANE_ID --replace
   ```
4. Retire the lane — **unless a `babysitter-v3` aggregate run dispatched you, where the head owns the park and runs this itself, so you must not:**
   ```bash
   just channel-clean "pr-<PR_NUMBER>"
   stint unclaim <id>
   ```

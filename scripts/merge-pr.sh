#!/usr/bin/env bash
# merge-pr.sh — PLEXI squash-merge pipeline
#
# Usage:
#   scripts/merge-pr.sh <PR>                   full flow (rebase→squash→sync→cleanup→close)
#   scripts/merge-pr.sh <PR> no-issue          full flow for a standalone PR — skips issue/stint close
#   scripts/merge-pr.sh rebase <BRANCH>        rebase feature branch on origin/alpha + force-push
#   scripts/merge-pr.sh squash <PR>            squash-merge only
#   scripts/merge-pr.sh sync                   reset local alpha to origin/alpha (safe — fails if unexpected commits)
#   scripts/merge-pr.sh cleanup <PR> <BRANCH>  remove all local and remote branch artifacts
#   scripts/merge-pr.sh close <ISSUE> <PR>     strip pipeline labels + close issue + append ship log
#   scripts/merge-pr.sh close-stints <PR> <ID...>
#
# Call sub-steps directly to resume after a failure (e.g. rebase conflict).
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"

do_rebase() {
    local BRANCH="$1"
    git fetch origin
    local BEHIND
    BEHIND=$(git -C "worktrees/$BRANCH" rev-list HEAD..origin/alpha --count 2>/dev/null || echo 0)
    echo "==> Rebasing $BRANCH ($BEHIND commits behind origin/alpha)"
    git -C "worktrees/$BRANCH" rebase origin/alpha
    git -C "worktrees/$BRANCH" push --force-with-lease origin HEAD
    # GitHub merge state lags after a force-push — wait before merging
    echo "==> Waiting for GitHub to register push..."
    sleep 10
}

do_squash() {
    local PR="$1"
    echo "==> Squash-merging PR #$PR"
    gh pr merge "$PR" --squash
}

do_sync() {
    echo "==> Syncing local alpha to origin/alpha"
    git fetch origin
    # Safety check: local alpha should only have the claim commit ahead of origin.
    # If there are more, something unexpected happened — fail loud rather than destroy.
    LOCAL_AHEAD=$(git rev-list origin/alpha..HEAD --count)
    if [ "$LOCAL_AHEAD" -gt 1 ]; then
        echo "ERROR: local alpha has $LOCAL_AHEAD commits not on origin/alpha (expected at most 1 claim commit)"
        git log origin/alpha..HEAD --oneline
        echo "Investigate before proceeding. To force-sync: git reset --hard origin/alpha"
        exit 1
    fi
    git reset --hard origin/alpha
}

do_cleanup() {
    local PR="$1"
    local BRANCH="$2"
    local WORKTREE_PATH="$ROOT/worktrees/$BRANCH"
    echo "==> Cleaning up PR #$PR artifacts"
    rm -f "test_pr${PR}.py"
    just channel-clean "pr-${PR}" || echo "warn: channel-clean pr-${PR} returned non-zero (continuing)" >&2

    # `git worktree remove --force` routinely fails with "Directory not empty" —
    # it deregisters the worktree but leaves files (build artifacts, untracked
    # scratch) on disk. Under `set -e` that used to abort the whole script here,
    # *before* the issue/stint close ran, so the PR merged but the task silently
    # stayed in-progress. Self-heal instead of aborting: drop the leftovers and
    # only fail if the directory genuinely survives.
    if git worktree list --porcelain | grep -Fqx "worktree $WORKTREE_PATH"; then
        git worktree remove --force "$WORKTREE_PATH" || true
    fi
    git worktree prune
    if [ -e "$WORKTREE_PATH" ]; then
        echo "==> Worktree dir survived removal (usually 'Directory not empty') — clearing: $WORKTREE_PATH"
        rm -rf "$WORKTREE_PATH"
        git worktree prune
    fi
    if [ -e "$WORKTREE_PATH" ]; then
        echo "ERROR: cleanup left worktree directory after rm -rf: $WORKTREE_PATH" >&2
        exit 1
    fi
    if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
        git branch -D "$BRANCH"
    fi
    if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
        echo "ERROR: cleanup left local branch: $BRANCH" >&2
        exit 1
    fi
    if git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
        git push origin --delete "$BRANCH"
    fi
    if git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
        echo "ERROR: cleanup left remote branch: origin/$BRANCH" >&2
        exit 1
    fi
    if git worktree list --porcelain | grep -Fqx "worktree $WORKTREE_PATH"; then
        echo "ERROR: cleanup left registered worktree: $WORKTREE_PATH" >&2
        exit 1
    fi
    echo "==> Reaping finished merged worktrees (clean + fully pushed only)"
    just reap-merged-worktrees 2>/dev/null || true
    echo "==> Sweeping stale worktree build artifacts"
    just clean-stale-targets 2>/dev/null || true
    echo "==> Sweeping closed/merged PR channel installs"
    just channel-clean-merged 2>/dev/null || true
}

do_close() {
    local ISSUE="$1"
    local PR="$2"
    local VERSION
    VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')

    echo "==> Closing issue #$ISSUE (current alpha v$VERSION)"
    gh issue edit "$ISSUE" \
        --remove-label "pipeline:merge" \
        --remove-label "pipeline:validate" \
        --remove-label "pipeline:open-pr" \
        --remove-label "pipeline:implement" \
        --remove-label "in progress" 2>/dev/null || true

    gh issue close "$ISSUE" --comment "Closed by PR #${PR} — merged to alpha (current version v${VERSION})"

    CURRENT_BODY=$(gh issue view "$ISSUE" --json body --jq '.body')
    gh issue edit "$ISSUE" --body "$(printf '%s\n**Merged:** PR #%s → alpha (current version v%s, %s)' \
        "$CURRENT_BODY" "$PR" "$VERSION" "$(date +%Y-%m-%d)")"
}

task_file_for_stint() {
    local STINT="$1"
    if [ ! -d .stint/tasks ]; then
        echo "ERROR: .stint/tasks not found in $ROOT" >&2
        echo "Run merge-pr from the canonical alpha checkout that owns local stint state, not a feature worktree without .stint/." >&2
        return 1
    fi
    find .stint/tasks -maxdepth 1 -name "${STINT}-*.md" -print -quit
}

stint_ids_from_body() {
    # Explicit directives only — never free prose. A prose mention such as
    # "stint-0568-shaped" is useful writing, not a close directive.
    #   Stint: 0560, 0561          (also `Stints:`, optionally bulleted/bolded)
    #   Closes stint 0560          (closing keyword + `stint` + id)
    local PR_BODY="$1"
    {
        printf '%s\n' "$PR_BODY" \
            | grep -Ei '^[[:space:]>*_-]*stints?[[:space:]*_]*:' \
            | sed -E 's/^[^:]*://' \
            | grep -Eo '[0-9]{4}' || true
        printf '%s\n' "$PR_BODY" \
            | grep -Eio '(close[sd]?|fix(e[sd])?|resolve[sd]?) +stint[ #-]*[0-9]{4}' \
            | grep -Eo '[0-9]{4}$' || true
    }
}

resolve_stints() {
    local BRANCH="$1"
    local PR_BODY="${2:-}"
    local STINT
    {
        printf '%s\n' "$BRANCH" | grep -Eo 'stint-[0-9-]+' | grep -Eo '[0-9]{4}' || true
        stint_ids_from_body "$PR_BODY"
    } | sort -u | while IFS= read -r STINT; do
        [ -n "$STINT" ] || continue
        local TASK_FILE
        TASK_FILE=$(task_file_for_stint "$STINT") || return 1
        if [ -z "$TASK_FILE" ]; then
            echo "ERROR: PR references stint task $STINT, but no .stint/tasks/${STINT}-*.md exists" >&2
            return 1
        fi
        printf '%s\n' "$STINT"
    done
}

join_by() {
    local IFS="$1"
    shift
    printf '%s\n' "$*"
}

# describe_closures <issue-or-empty> [stint-id ...]
# "issue #N", "stint task(s) A, B", or both joined with " and ".
describe_closures() {
    local ISSUE="${1:-}"
    shift || true
    local PARTS=""
    if [ -n "$ISSUE" ]; then
        PARTS="issue #$ISSUE"
    fi
    if [ "$#" -gt 0 ]; then
        [ -z "$PARTS" ] || PARTS="$PARTS and "
        PARTS="${PARTS}stint task(s) $(join_by ", " "$@")"
    fi
    printf '%s\n' "$PARTS"
}

do_close_stints() {
    local PR="$1"
    shift
    local VERSION
    local STINTS
    local STINT

    VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
    STINTS=$(join_by ", " "$@")

    echo "==> Closing stint task(s): $STINTS"
    for STINT in "$@"; do
        stint done "$STINT"
    done

    echo "==> Closed stint task(s) $STINTS (current alpha v$VERSION)"
}

resolve_issue() {
    local PR="$1"
    local BRANCH="$2"
    local PR_BODY="${3:-}"
    local ISSUE

    ISSUE=$(gh pr view "$PR" --json closingIssuesReferences \
        --jq '.closingIssuesReferences[0].number // empty')
    if [ -n "$ISSUE" ]; then
        printf '%s\n' "$ISSUE"
        return
    fi

    ISSUE=$(printf '%s\n' "$PR_BODY" \
        | grep -Eio '(close[sd]?|fix(e[sd])?|resolve[sd]?) +#[0-9]+' \
        | head -1 \
        | grep -oE '[0-9]+' || true)
    if [ -n "$ISSUE" ]; then
        printf '%s\n' "$ISSUE"
        return
    fi

    case "$BRANCH" in
        feature/[0-9]*-*|fix/[0-9]*-*)
            printf '%s\n' "$BRANCH" | grep -oE '[0-9]+' | head -1
            return
            ;;
    esac

    return 1
}

test_resolve_issue() {
    gh() {
        if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
            if printf '%s\n' "$*" | grep -q 'closingIssuesReferences'; then
                printf '\n'
            else
                return 1
            fi
        fi
    }

    local actual
    actual=$(resolve_issue 2171 "feature/stint-0004-file-explorer-multi-select-ops" "Closes #2138")
    [ "$actual" = "2138" ] || {
        echo "expected PR body closing issue 2138, got '$actual'" >&2
        return 1
    }

    actual=$(resolve_issue 2155 "feature/2155-short-name" "")
    [ "$actual" = "2155" ] || {
        echo "expected numeric branch issue 2155, got '$actual'" >&2
        return 1
    }

    if resolve_issue 4 "feature/stint-0004-short-name" "" >/dev/null 2>&1; then
        echo "expected unresolved stint branch without PR body to fail" >&2
        return 1
    fi

    echo "resolve_issue tests passed"
}

test_resolve_stints() {
    local TMP actual
    TMP=$(mktemp -d)
    mkdir -p "$TMP/.stint/tasks"
    local ID
    for ID in 0241 0285 0327 0560 0561 0568; do
        : > "$TMP/.stint/tasks/${ID}-fixture.md"
    done

    # resolve_stints reads .stint/tasks relative to cwd (the canonical checkout).
    (
        cd "$TMP"

        actual=$(resolve_stints "feature/stint-0241-0285-0327-packages-trust" "")
        [ "$actual" = "$(printf '0241\n0285\n0327')" ] || {
            echo "expected stint ids from branch, got '$actual'" >&2
            exit 1
        }

        actual=$(resolve_stints "feature/packages-trust" "Stint: 0241, 0285, 0327")
        [ "$actual" = "$(printf '0241\n0285\n0327')" ] || {
            echo "expected stint ids from explicit 'Stint:' body line, got '$actual'" >&2
            exit 1
        }

        actual=$(resolve_stints "feature/packages-trust" "Closes stint 0560")
        [ "$actual" = "0560" ] || {
            echo "expected stint id from closing keyword, got '$actual'" >&2
            exit 1
        }

        # Prose mention of an unrelated stint must NOT be treated as a close directive
        # (PR #2502 closed 0568 because its body said "stint-0568-shaped").
        actual=$(resolve_stints "feature/stint-0560-merge-pr" "The squad is stint-0568-shaped, see stint 0561 and Stint 0285 notes.")
        [ "$actual" = "0560" ] || {
            echo "expected prose-mentioned stints to be ignored (only 0560), got '$actual'" >&2
            exit 1
        }

        if resolve_stints "feature/stint-9999-missing" "" >/dev/null 2>&1; then
            echo "expected missing stint task to fail" >&2
            exit 1
        fi

        # Strictness holds even when an issue is also present: a named-but-missing
        # task is still an error.
        if resolve_stints "feature/packages-trust" "Closes #2138
Stint: 9999" >/dev/null 2>&1; then
            echo "expected missing explicit stint task to fail" >&2
            exit 1
        fi
    ) || return 1

    echo "resolve_stints tests passed"
}

# Regression (stint 0560): a PR with BOTH a closing issue and stint ids resolves
# both, and an unrelated prose mention resolves neither.
test_resolve_both() {
    gh() {
        if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
            if printf '%s\n' "$*" | grep -q 'closingIssuesReferences'; then
                printf '\n'
            else
                return 1
            fi
        fi
    }

    local TMP actual BODY
    TMP=$(mktemp -d)
    mkdir -p "$TMP/.stint/tasks"
    local ID
    for ID in 0560 0561 0568; do
        : > "$TMP/.stint/tasks/${ID}-fixture.md"
    done
    BODY="Closes #2138
Stint: 0561
The three-pane squad was stint-0568-shaped."

    (
        cd "$TMP"
        actual=$(resolve_issue 2497 "feature/stint-0560-merge-pr" "$BODY")
        [ "$actual" = "2138" ] || {
            echo "expected issue 2138 alongside stints, got '$actual'" >&2
            exit 1
        }
        actual=$(resolve_stints "feature/stint-0560-merge-pr" "$BODY")
        [ "$actual" = "$(printf '0560\n0561')" ] || {
            echo "expected stints 0560+0561 alongside issue (not prose 0568), got '$actual'" >&2
            exit 1
        }
        actual=$(describe_closures 2138 0560 0561)
        [ "$actual" = "issue #2138 and stint task(s) 0560,0561" ] || {
            echo "unexpected closure description '$actual'" >&2
            exit 1
        }
        actual=$(describe_closures "" 0560)
        [ "$actual" = "stint task(s) 0560" ] || {
            echo "unexpected stint-only description '$actual'" >&2
            exit 1
        }
    ) || return 1

    echo "resolve_both tests passed"
}

# Regression: a worktree dir that survives `git worktree remove --force` (the
# routine "Directory not empty" case) must be cleared, not abort the run before
# the issue/stint close. See do_cleanup.
test_cleanup_survivor() {
    local TMP
    TMP=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf '$TMP'" RETURN

    local WORKTREE_PATH="$TMP/worktrees/feature/fake-branch"
    mkdir -p "$WORKTREE_PATH"
    echo "leftover build artifact" > "$WORKTREE_PATH/stale.o"

    # Mimic do_cleanup's survivor branch in isolation: removal already failed and
    # left files behind, so the directory is still present at this point.
    if [ -e "$WORKTREE_PATH" ]; then
        rm -rf "$WORKTREE_PATH"
    fi

    if [ -e "$WORKTREE_PATH" ]; then
        echo "expected leftover worktree dir to be cleared, still present" >&2
        return 1
    fi

    echo "cleanup survivor test passed"
}

# --- Dispatch ---
CMD="${1:-}"
case "$CMD" in
    test-resolve-issue) test_resolve_issue ;;
    test-resolve-stints) test_resolve_stints ;;
    test-resolve-both) test_resolve_both ;;
    test-cleanup-survivor) test_cleanup_survivor ;;
    rebase)   do_rebase "${2:?BRANCH required}" ;;
    squash)   do_squash "${2:?PR required}" ;;
    sync)     do_sync ;;
    cleanup)  do_cleanup "${2:?PR required}" "${3:?BRANCH required}" ;;
    close)    do_close "${2:?ISSUE required}" "${3:?PR required}" ;;
    close-stints)
        PR="${2:?PR required}"
        shift 2
        [ "$#" -gt 0 ] || {
            echo "ERROR: at least one stint task id is required" >&2
            exit 1
        }
        do_close_stints "$PR" "$@"
        ;;
    *)
        PR="${1:?PR number required}"
        NO_ISSUE=0
        case "${2:-}" in
            "") ;;
            no-issue|--no-issue) NO_ISSUE=1 ;;
            *)
                echo "ERROR: unknown argument '${2}' (did you mean 'no-issue'?)" >&2
                exit 1
                ;;
        esac

        INFO=$(gh pr view "$PR" --json headRefName,state,body --jq '{branch: .headRefName, state: .state, body: .body}')
        BRANCH=$(echo "$INFO" | jq -r .branch)
        STATE=$(echo "$INFO" | jq -r .state)
        PR_BODY=$(echo "$INFO" | jq -r '.body // ""')
        ISSUE=""
        STINTS=""
        STINT_ARGS=()
        if [ "$NO_ISSUE" -eq 0 ]; then
            ISSUE=$(resolve_issue "$PR" "$BRANCH" "$PR_BODY" || true)
            # Resolved independently of the issue: a PR routinely has both. A named
            # stint with no task file is a hard error (set -e), issue found or not.
            STINTS=$(resolve_stints "$BRANCH" "$PR_BODY")
            if [ -z "$ISSUE" ] && [ -z "$STINTS" ]; then
                echo "ERROR: could not resolve GitHub issue or stint tasks for PR #$PR ($BRANCH)" >&2
                echo "Add a closing keyword like 'Closes #1234', use a numeric feature/fix branch, or include stint ids in the branch or a 'Stint: 0123, 0124' line in the PR body." >&2
                echo "For a standalone follow-up PR with nothing to close, re-run: just merge-pr $PR no-issue" >&2
                exit 1
            fi
            while IFS= read -r STINT; do
                if [ -n "$STINT" ]; then
                    STINT_ARGS+=("$STINT")
                fi
            done <<< "$STINTS"
        fi

        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> PR #$PR: $BRANCH (standalone — no issue/stint to close, state: $STATE)"
        else
            echo "==> PR #$PR: $BRANCH ($(describe_closures "$ISSUE" "${STINT_ARGS[@]+"${STINT_ARGS[@]}"}"), state: $STATE)"
        fi

        # Fail fast if root worktree has uncommitted changes
        DIRTY=$(git status --porcelain | grep -v "^??" || true)
        if [ -n "$DIRTY" ]; then
            echo "ERROR: root worktree has uncommitted changes — commit or restore before merging:"
            echo "$DIRTY"
            exit 1
        fi

        if [ "$STATE" != "MERGED" ]; then
            do_rebase "$BRANCH"
            do_squash "$PR"
        else
            echo "==> PR already merged — skipping rebase and squash"
        fi

        do_sync
        do_cleanup "$PR" "$BRANCH"
        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> Skipping issue/stint close (no-issue)"
        else
            if [ -n "$ISSUE" ]; then
                do_close "$ISSUE" "$PR"
            fi
            if [ "${#STINT_ARGS[@]}" -gt 0 ]; then
                do_close_stints "$PR" "${STINT_ARGS[@]}"
            fi
        fi

        VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
        echo ""
        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> Done: PR #$PR merged → alpha (current version v$VERSION, standalone — nothing closed)"
        else
            echo "==> Done: PR #$PR merged → alpha (current version v$VERSION), $(describe_closures "$ISSUE" "${STINT_ARGS[@]+"${STINT_ARGS[@]}"}") closed"
        fi
        ;;
esac

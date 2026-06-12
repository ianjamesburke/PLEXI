#!/usr/bin/env bash
# merge-pr.sh — PLEXI squash-merge pipeline
#
# Usage:
#   scripts/merge-pr.sh <PR>                   full flow (rebase→squash→sync→cleanup→bump→close)
#   scripts/merge-pr.sh <PR> no-issue          full flow for a standalone PR — skips issue/stint close
#   scripts/merge-pr.sh rebase <BRANCH>        rebase feature branch on origin/alpha + force-push
#   scripts/merge-pr.sh squash <PR>            squash-merge only
#   scripts/merge-pr.sh sync                   reset local alpha to origin/alpha (safe — fails if unexpected commits)
#   scripts/merge-pr.sh cleanup <PR> <BRANCH>  channel-clean + wtp remove + remote branch delete
#   scripts/merge-pr.sh bump                   just bump + git push
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
    if [ "$BEHIND" -gt 0 ]; then
        echo "==> Rebasing $BRANCH ($BEHIND commits behind origin/alpha)"
        git -C "worktrees/$BRANCH" rebase origin/alpha
        git -C "worktrees/$BRANCH" push --force-with-lease origin HEAD
        # GitHub merge state lags after a force-push — wait before merging
        echo "==> Waiting for GitHub to register push..."
        sleep 10
    else
        echo "==> $BRANCH is up to date with origin/alpha"
    fi
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
    echo "==> Cleaning up PR #$PR artifacts"
    rm -f "test_pr${PR}.py"
    just channel-clean "pr-${PR}" 2>/dev/null || true
    wtp remove "$BRANCH" --force --with-branch 2>/dev/null || true
    git push origin --delete "$BRANCH" 2>/dev/null || true
}

do_bump() {
    echo "==> Bumping version"
    just bump
    git push
}

do_close() {
    local ISSUE="$1"
    local PR="$2"
    local VERSION
    VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')

    echo "==> Closing issue #$ISSUE (v$VERSION)"
    gh issue edit "$ISSUE" \
        --remove-label "pipeline:merge" \
        --remove-label "pipeline:validate" \
        --remove-label "pipeline:open-pr" \
        --remove-label "pipeline:implement" \
        --remove-label "in progress" 2>/dev/null || true

    gh issue close "$ISSUE" --comment "Closed by PR #${PR} — verified on alpha v${VERSION}"

    CURRENT_BODY=$(gh issue view "$ISSUE" --json body --jq '.body')
    gh issue edit "$ISSUE" --body "$(printf '%s\n**Merged:** PR #%s → alpha v%s (%s)' \
        "$CURRENT_BODY" "$PR" "$VERSION" "$(date +%Y-%m-%d)")"
}

task_file_for_stint() {
    local STINT="$1"
    find .stint/tasks -maxdepth 1 -name "${STINT}-*.md" -print -quit
}

resolve_stints() {
    local BRANCH="$1"
    local PR_BODY="${2:-}"
    local STINT
    {
        printf '%s\n' "$BRANCH" | grep -Eo 'stint-[0-9-]+' | grep -Eo '[0-9]{4}' || true
        printf '%s\n' "$PR_BODY" | grep -Ei 'stint' | grep -Eo '[0-9]{4}' || true
    } | sort -u | while IFS= read -r STINT; do
        [ -n "$STINT" ] || continue
        if [ -z "$(task_file_for_stint "$STINT")" ]; then
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

    if git diff --quiet -- .stint/tasks; then
        echo "==> No stint task changes to commit"
        return
    fi

    git add .stint/tasks
    git commit -m "chore(stint): close tasks $STINTS after PR #$PR"
    git push
    echo "==> Closed stint task(s) $STINTS on alpha v$VERSION"
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
    local actual
    actual=$(resolve_stints "feature/stint-0015-0016-0017-packages-trust" "")
    [ "$actual" = "$(printf '0015\n0016\n0017')" ] || {
        echo "expected stint ids from branch, got '$actual'" >&2
        return 1
    }

    actual=$(resolve_stints "feature/packages-trust" "Stint tasks 0015, 0016, 0017")
    [ "$actual" = "$(printf '0015\n0016\n0017')" ] || {
        echo "expected stint ids from PR body, got '$actual'" >&2
        return 1
    }

    if resolve_stints "feature/stint-9999-missing" "" >/dev/null 2>&1; then
        echo "expected missing stint task to fail" >&2
        return 1
    fi

    echo "resolve_stints tests passed"
}

# --- Dispatch ---
CMD="${1:-}"
case "$CMD" in
    test-resolve-issue) test_resolve_issue ;;
    test-resolve-stints) test_resolve_stints ;;
    rebase)   do_rebase "${2:?BRANCH required}" ;;
    squash)   do_squash "${2:?PR required}" ;;
    sync)     do_sync ;;
    cleanup)  do_cleanup "${2:?PR required}" "${3:?BRANCH required}" ;;
    bump)     do_bump ;;
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
            if [ -z "$ISSUE" ]; then
                STINTS=$(resolve_stints "$BRANCH" "$PR_BODY")
                if [ -z "$STINTS" ]; then
                    echo "ERROR: could not resolve GitHub issue or stint tasks for PR #$PR ($BRANCH)" >&2
                    echo "Add a closing keyword like 'Closes #1234', use a numeric feature/fix branch, or include stint ids in the branch/body." >&2
                    echo "For a standalone follow-up PR with nothing to close, re-run: just merge-pr $PR no-issue" >&2
                    exit 1
                fi
                while IFS= read -r STINT; do
                    [ -n "$STINT" ] && STINT_ARGS+=("$STINT")
                done <<< "$STINTS"
            fi
        fi

        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> PR #$PR: $BRANCH (standalone — no issue/stint to close, state: $STATE)"
        elif [ -n "$ISSUE" ]; then
            echo "==> PR #$PR: $BRANCH (issue #$ISSUE, state: $STATE)"
        else
            echo "==> PR #$PR: $BRANCH (stint tasks: $(join_by ", " "${STINT_ARGS[@]}"), state: $STATE)"
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
        do_bump
        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> Skipping issue/stint close (no-issue)"
        elif [ -n "$ISSUE" ]; then
            do_close "$ISSUE" "$PR"
        else
            do_close_stints "$PR" "${STINT_ARGS[@]}"
        fi

        VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
        echo ""
        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> Done: PR #$PR merged → alpha v$VERSION (standalone — nothing closed)"
        elif [ -n "$ISSUE" ]; then
            echo "==> Done: PR #$PR merged → alpha v$VERSION, issue #$ISSUE closed"
        else
            echo "==> Done: PR #$PR merged → alpha v$VERSION, stint task(s) $(join_by ", " "${STINT_ARGS[@]}") closed"
        fi
        ;;
esac

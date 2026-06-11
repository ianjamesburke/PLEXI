#!/usr/bin/env bash
# merge-pr.sh — PLEXI squash-merge pipeline
#
# Usage:
#   scripts/merge-pr.sh <PR>                   full flow (rebase→squash→sync→cleanup→bump→close)
#   scripts/merge-pr.sh rebase <BRANCH>        rebase feature branch on origin/alpha + force-push
#   scripts/merge-pr.sh squash <PR>            squash-merge only
#   scripts/merge-pr.sh sync                   reset local alpha to origin/alpha (safe — fails if unexpected commits)
#   scripts/merge-pr.sh cleanup <PR> <BRANCH>  channel-clean + wtp remove + remote branch delete
#   scripts/merge-pr.sh bump                   just bump + git push
#   scripts/merge-pr.sh close <ISSUE> <PR>     strip pipeline labels + close issue + append ship log
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

# --- Dispatch ---
CMD="${1:-}"
case "$CMD" in
    rebase)   do_rebase "${2:?BRANCH required}" ;;
    squash)   do_squash "${2:?PR required}" ;;
    sync)     do_sync ;;
    cleanup)  do_cleanup "${2:?PR required}" "${3:?BRANCH required}" ;;
    bump)     do_bump ;;
    close)    do_close "${2:?ISSUE required}" "${3:?PR required}" ;;
    *)
        PR="${1:?PR number required}"

        INFO=$(gh pr view "$PR" --json headRefName,state --jq '{branch: .headRefName, state: .state}')
        BRANCH=$(echo "$INFO" | jq -r .branch)
        STATE=$(echo "$INFO" | jq -r .state)
        ISSUE=$(echo "$BRANCH" | grep -oE '[0-9]+' | head -1)

        echo "==> PR #$PR: $BRANCH (issue #$ISSUE, state: $STATE)"

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
        do_close "$ISSUE" "$PR"

        VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
        echo ""
        echo "==> Done: PR #$PR merged → alpha v$VERSION, issue #$ISSUE closed"
        ;;
esac

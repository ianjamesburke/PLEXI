#!/usr/bin/env bash
# merge-pr.sh — PLEXI squash-merge pipeline
#
# Usage:
#   scripts/merge-pr.sh <PR>                   full flow (squash→fast-forward alpha→lane cleanup→close)
#   scripts/merge-pr.sh <PR> no-issue          standalone PR — skips issue/stint close
#   scripts/merge-pr.sh sync                   fast-forward alpha without discarding local work
#   scripts/merge-pr.sh cleanup <PR> <BRANCH> <HEAD>  remove pr-<PR> channel, clean lane worktree and branches
#   scripts/merge-pr.sh close <ISSUE> <PR>     strip pipeline labels + close issue + append ship log
#   scripts/merge-pr.sh close-stints <PR> [--partial <ID>]... [ID...]   done ID; release (unclaim) --partial ID
#
# Call sub-steps directly to resume after a failure (e.g. rebase conflict).
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"

# Acquire before entering full-flow landing.
case "${1:-}" in
    --alpha-lock-inner)
        shift
        python3 -I "$SCRIPT_DIR/alpha_lock.py" --verify
        ;;
    [0-9]*)
        exec python3 -I "$SCRIPT_DIR/alpha_lock.py" bash "$SCRIPT_DIR/merge-pr.sh" --alpha-lock-inner "$@"
        ;;
esac

do_merge() {
    local PR="$1"
    echo "==> Squash-merging PR #$PR"
    if ! gh pr merge "$PR" --squash; then
        echo "ERROR: GitHub could not squash-merge PR #$PR (conflict with alpha or a branch rule). Nothing local changed; resolve it on the PR branch, then rerun: just merge-pr $PR" >&2
        return 1
    fi
}

do_sync() {
    if [ "$(git branch --show-current)" != alpha ]; then
        echo "ERROR: sync must run from the canonical alpha checkout." >&2
        return 1
    fi
    echo "==> Fast-forwarding local alpha to origin/alpha (local changes are preserved)"
    git fetch origin alpha
    if [ "$(git rev-list --count origin/alpha..HEAD)" != 0 ]; then
        echo "ERROR: local alpha contains commits absent from origin; preserve and reconcile them before landing." >&2
        return 1
    fi
    if ! git merge --ff-only origin/alpha; then
        echo "ERROR: alpha could not fast-forward; local edits and commits were preserved. Resolve the conflict or divergence and rerun; do not reset." >&2
        return 1
    fi
}

assert_clean_lane() {
    local BRANCH="$1" WORKTREE_PATH="$ROOT/worktrees/$1"
    if git worktree list --porcelain | grep -Fqx "worktree $WORKTREE_PATH"; then
        if [ -n "$(git -C "$WORKTREE_PATH" status --porcelain)" ]; then
            echo "ERROR: lane worktree has uncommitted content; preserve or commit it before cleanup: $WORKTREE_PATH" >&2
            git -C "$WORKTREE_PATH" status --short >&2
            return 1
        fi
    elif [ -e "$WORKTREE_PATH" ]; then
        echo "ERROR: unregistered lane path exists; inspect it before cleanup: $WORKTREE_PATH" >&2
        return 1
    fi
}

# Heads the PR branch held before force-pushes that led to PR_HEAD, newest
# first, from GitHub's HeadRefForcePushedEvent record. A server-side rebase is
# such a force-push; the local lane still holds the pre-rebase head.
prior_pr_heads() {
    local PR="$1" PR_HEAD="$2" EVENTS
    EVENTS=$(gh api graphql -F owner='{owner}' -F name='{repo}' -F number="$PR" -f query='
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      timelineItems(itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT], last: 100) {
        nodes { ... on HeadRefForcePushedEvent { beforeCommit { oid } afterCommit { oid } } }
      }
    }
  }
}') || {
        echo "ERROR: could not read force-push history of PR #$PR from GitHub; no cleanup performed." >&2
        return 1
    }
    printf '%s' "$EVENTS" | jq -r --arg head "$PR_HEAD" '
        [.data.repository.pullRequest.timelineItems.nodes[]
         | {b: .beforeCommit.oid, a: .afterCommit.oid}]
        | reverse
        | reduce .[] as $e ({cur: $head, out: []};
            if $e.a == .cur and $e.b != null then {cur: $e.b, out: (.out + [$e.b])} else . end)
        | .out[]'
}

lane_head_matches() {
    local HEAD="$1" PR_HEAD="$2" PRIOR="$3"
    [ -z "$HEAD" ] || [ "$HEAD" = "$PR_HEAD" ] || printf '%s\n' "$PRIOR" | grep -Fqx "$HEAD"
}

do_cleanup() {
    local PR="$1" BRANCH="$2" LANE_HEAD="$3" WORKTREE_PATH="$ROOT/worktrees/$2"
    local INFO STATE PR_BRANCH PR_HEAD LOCAL_HEAD REMOTE_HEAD PRIOR=""
    INFO=$(gh pr view "$PR" --json state,headRefName,headRefOid)
    STATE=$(printf '%s' "$INFO" | jq -r .state)
    PR_BRANCH=$(printf '%s' "$INFO" | jq -r .headRefName)
    PR_HEAD=$(printf '%s' "$INFO" | jq -r .headRefOid)
    if [ "$STATE" != MERGED ] || [ "$PR_BRANCH" != "$BRANCH" ] || ! [[ "$LANE_HEAD" =~ ^[0-9a-f]{40}$ ]] || [ "$PR_HEAD" != "$LANE_HEAD" ]; then
        echo "ERROR: cannot establish merged PR #$PR and its exact lane head; no cleanup performed." >&2
        return 1
    fi
    assert_clean_lane "$BRANCH"
    LOCAL_HEAD=$(git rev-parse -q --verify "refs/heads/$BRANCH" || true)
    REMOTE_HEAD=$(git ls-remote --heads origin "refs/heads/$BRANCH" | awk '{print $1}')
    if ! lane_head_matches "$LOCAL_HEAD" "$PR_HEAD" "" || ! lane_head_matches "$REMOTE_HEAD" "$PR_HEAD" ""; then
        PRIOR=$(prior_pr_heads "$PR" "$PR_HEAD") || return 1
        if ! lane_head_matches "$LOCAL_HEAD" "$PR_HEAD" "$PRIOR" || ! lane_head_matches "$REMOTE_HEAD" "$PR_HEAD" "$PRIOR"; then
            echo "ERROR: lane branch has moved since its PR (local ${LOCAL_HEAD:-none}, remote ${REMOTE_HEAD:-none}, merged head $PR_HEAD, not a pre-rebase head); preserve it and inspect before cleanup." >&2
            return 1
        fi
        echo "==> Lane holds pre-rebase head of PR #$PR (local ${LOCAL_HEAD:-none}, remote ${REMOTE_HEAD:-none}); GitHub force-pushed it to merged head $PR_HEAD"
        # A force-push can drop commits, so the timeline does not prove the old
        # head's work landed. Keep every old head under a durable local ref.
        if [ -n "$REMOTE_HEAD" ] && [ "$REMOTE_HEAD" != "$PR_HEAD" ]; then
            git fetch -q origin "refs/heads/$BRANCH"
            if [ "$(git rev-parse FETCH_HEAD)" != "$REMOTE_HEAD" ]; then
                echo "ERROR: remote lane branch moved while preserving it; no cleanup performed." >&2
                return 1
            fi
        fi
        local HEAD_SHA
        for HEAD_SHA in $(printf '%s\n%s\n' "$LOCAL_HEAD" "$REMOTE_HEAD" | sort -u); do
            [ "$HEAD_SHA" != "$PR_HEAD" ] || continue
            git update-ref "refs/plexi-preserved/pr-$PR/$HEAD_SHA" "$HEAD_SHA"
            echo "==> Preserved pre-rebase head $HEAD_SHA at refs/plexi-preserved/pr-$PR/$HEAD_SHA (restore: git branch <name> refs/plexi-preserved/pr-$PR/$HEAD_SHA)"
        done
    fi
    echo "==> Cleaning up PR #$PR lane artifacts"
    just channel-clean "pr-${PR}"
    if git worktree list --porcelain | grep -Fqx "worktree $WORKTREE_PATH"; then
        # Git removes ignored files even without --force. Keep the entire lane
        # when they exist, and allow the merged stint to close truthfully.
        local IGNORED
        IGNORED=$(python3 "$SCRIPT_DIR/cleanup-ignored.py" "$WORKTREE_PATH") || {
            echo "ERROR: could not safely classify ignored lane content; retaining lane." >&2
            return 1
        }
        if [ -n "$IGNORED" ]; then
            echo "NOTICE: PR #$PR merged; retaining lane and branches to preserve ignored content: $WORKTREE_PATH" >&2
            printf '%s\n' "$IGNORED" >&2
            echo "Preserve or deliberately handle those files, then run: just merge-cleanup $PR $BRANCH $LANE_HEAD" >&2
            return 0
        fi
        echo "==> Retiring lane with recognized generated artifacts (if present)"
        git worktree remove "$WORKTREE_PATH"
    fi
    # Compare-and-delete: a commit made on the lane after the checks above
    # keeps the branch. update-ref does not check worktrees, so refuse first.
    if git worktree list --porcelain | grep -Fqx "branch refs/heads/$BRANCH"; then
        echo "ERROR: lane branch $BRANCH is checked out in another worktree; branch kept, remote not deleted." >&2
        return 1
    fi
    if [ -n "$LOCAL_HEAD" ] && ! git update-ref -d "refs/heads/$BRANCH" "$LOCAL_HEAD"; then
        echo "ERROR: lane branch $BRANCH moved during cleanup (expected $LOCAL_HEAD); branch kept, remote not deleted." >&2
        return 1
    fi
    if git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
        git push --force-with-lease="refs/heads/$BRANCH:$REMOTE_HEAD" origin ":refs/heads/$BRANCH"
    fi
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

# A `Stint-partial: 0560, 0561` line (same tolerance as `Stint:`) names stints
# this PR delivers only a slice of: merge releases their claim, never closes them.
partial_ids_from_body() {
    local PR_BODY="$1"
    printf '%s\n' "$PR_BODY" \
        | grep -Ei '^[[:space:]>*_-]*stint-partial[[:space:]*_]*:' \
        | sed -E 's/^[^:]*://' \
        | grep -Eo '[0-9]{4}' | sort -u || true
}

require_task_file() {
    local STINT="$1"
    local TASK_FILE
    TASK_FILE=$(task_file_for_stint "$STINT") || return 1
    if [ -z "$TASK_FILE" ]; then
        echo "ERROR: PR references stint task $STINT, but no .stint/tasks/${STINT}-*.md exists" >&2
        return 1
    fi
}

# Stints to close: branch ids and `Stint:` ids, minus any id named partial.
resolve_stints() {
    local BRANCH="$1"
    local PR_BODY="${2:-}"
    local PARTIAL STINT
    PARTIAL=$(partial_ids_from_body "$PR_BODY")
    {
        printf '%s\n' "$BRANCH" | grep -Eo 'stint-[0-9-]+' | grep -Eo '[0-9]{4}' || true
        stint_ids_from_body "$PR_BODY"
    } | sort -u | while IFS= read -r STINT; do
        [ -n "$STINT" ] || continue
        if grep -qx "$STINT" <<< "$PARTIAL"; then
            continue
        fi
        require_task_file "$STINT" || return 1
        printf '%s\n' "$STINT"
    done
}

# assert_partial_releasable <id>: the task exists and its `## Remaining` section
# says what is left. Runs before the remote squash so a refusal leaves nothing merged.
assert_partial_releasable() {
    local STINT="$1"
    local BODY
    require_task_file "$STINT" || return 1
    BODY=$(stint show "$STINT" --json | jq -r '.body // ""') || {
        echo "ERROR: stint show $STINT --json failed while checking partial stint $STINT" >&2
        return 1
    }
    if ! printf '%s\n' "$BODY" | awk '
        /^##[[:space:]]+Remaining[[:space:]]*$/ { inside = 1; next }
        /^##[[:space:]]/ { inside = 0 }
        inside && /[^[:space:]]/ { found = 1 }
        END { exit found ? 0 : 1 }'; then
        echo "ERROR: PR marks stint $STINT partial, but its task has no non-empty '## Remaining' section." >&2
        echo "Add '## Remaining' to stint $STINT (stint edit $STINT) listing the scope still open, or close it with a 'Stint: $STINT' line." >&2
        return 1
    fi
}

# Partial ids that pass assert_partial_releasable, one per line.
resolve_partial_stints() {
    local PR_BODY="${1:-}"
    local STINT
    partial_ids_from_body "$PR_BODY" | while IFS= read -r STINT; do
        [ -n "$STINT" ] || continue
        assert_partial_releasable "$STINT" || return 1
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

# do_close_stints <PR> [--partial <ID>]... [ID...]
# Plain ids are marked done; --partial ids are released (unclaimed), never done.
do_close_stints() {
    local PR="$1"
    shift
    local VERSION STINT STATUS
    local CLOSE=() PARTIAL=()

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --partial) PARTIAL+=("${2:?--partial requires a stint id}"); shift 2 ;;
            *) CLOSE+=("$1"); shift ;;
        esac
    done
    if [ "${#CLOSE[@]}" -eq 0 ] && [ "${#PARTIAL[@]}" -eq 0 ]; then
        echo "ERROR: at least one stint task id is required" >&2
        return 1
    fi
    for STINT in "${PARTIAL[@]+"${PARTIAL[@]}"}"; do
        if [[ " ${CLOSE[*]+"${CLOSE[*]}"} " == *" $STINT "* ]]; then
            echo "ERROR: stint $STINT is named both closed and partial for PR #$PR" >&2
            return 1
        fi
        assert_partial_releasable "$STINT" || return 1
    done

    VERSION=$(grep '^version' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')

    for STINT in "${CLOSE[@]+"${CLOSE[@]}"}"; do
        stint done "$STINT"
        echo "==> Stint $STINT: closed (PR #$PR)"
    done
    for STINT in "${PARTIAL[@]+"${PARTIAL[@]}"}"; do
        STATUS=$(stint show "$STINT" --json | jq -r '.status') || {
            echo "ERROR: stint show $STINT --json failed while releasing partial stint $STINT" >&2
            return 1
        }
        if [ "$STATUS" = todo ]; then
            echo "==> Stint $STINT: already todo, nothing to release (partial, PR #$PR)"
            continue
        fi
        stint unclaim "$STINT"
        echo "==> Stint $STINT: released (partial, PR #$PR)"
    done

    echo "==> Stint closeout done for PR #$PR (current alpha v$VERSION)"
}

resolve_issue() {
    local PR="$1"
    # A branch name is not an explicit GitHub issue link.
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

    return 1
}

test_resolve_issue() {
    local ISSUE_REFERENCE="" actual BRANCH KEYWORD BODY
    gh() {
        if [ "$1" = "pr" ] && [ "$2" = "view" ] && [[ "$*" == *closingIssuesReferences* ]]; then
            printf '%s\n' "$ISSUE_REFERENCE"
        else
            echo "unexpected gh call: $*" >&2
            return 1
        fi
    }

    for BRANCH in feature/1347-x fix/1347-x feature/stint-1347-x; do
        for BODY in "" "Stint: 1347"; do
            actual=$(resolve_issue 3202 "$BRANCH" "$BODY" || true)
            [ -z "$actual" ] || {
                echo "expected no issue from branch $BRANCH, got '$actual'" >&2
                return 1
            }
        done
    done

    for KEYWORD in Closes Fixes Resolves; do
        actual=$(resolve_issue 3202 "feature/1347-x" "$KEYWORD #2138")
        [ "$actual" = "2138" ] || {
            echo "expected $KEYWORD to resolve issue 2138, got '$actual'" >&2
            return 1
        }
    done

    ISSUE_REFERENCE=2468
    actual=$(resolve_issue 3202 "feature/1347-x" "")
    [ "$actual" = "2468" ] || {
        echo "expected closingIssuesReferences issue 2468, got '$actual'" >&2
        return 1
    }

    # Exercise the real dispatch and closeout with all external effects stubbed.
    # Any gh issue call fails the flow, even the best-effort label edit.
    actual=$(
        gh() {
            case "$1 $2" in
                'pr view')
                    if [[ "$*" == *closingIssuesReferences* ]]; then
                        printf '\n'
                    else
                        printf '%s\n' '{"branch":"feature/1347-x","head":"0000000000000000000000000000000000000001","base":"alpha","state":"OPEN","body":"Stint: 1347"}'
                    fi
                    ;;
                'pr merge') echo "mock merge $*" ;;
                *) echo "unexpected gh call: $*" >&2; exit 1 ;;
            esac
        }
        do_sync() { :; }
        assert_clean_lane() { :; }
        do_cleanup() { :; }
        task_file_for_stint() { echo "$1-fixture.md"; }
        stint() { echo "mock stint $*"; }
        set -- 3202
        eval "$(sed -n '/^# --- Dispatch ---/,$p' "$SCRIPT_DIR/merge-pr.sh")"
    ) || return 1
    [[ "$actual" == *"mock merge pr merge 3202 --squash"* ]] &&
        [[ "$actual" == *"mock stint done 1347"* ]] || {
        echo "expected stint-only merge and closeout, got '$actual'" >&2
        return 1
    }

    echo "resolve_issue tests passed (branch rejection, explicit links, stint-only merge)"
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

# Stint 1690: `Stint-partial:` releases a claim instead of closing the stint, and
# refuses (before any squash) unless the stint has a non-empty `## Remaining`.
test_resolve_partial() {
    local TMP
    TMP=$(mktemp -d)
    mkdir -p "$TMP/.stint/tasks" "$TMP/fixtures"
    printf 'version = "0.0.0"\n' > "$TMP/Cargo.toml"
    fixture() { # <id> <status> <body>
        : > "$TMP/.stint/tasks/$1-fixture.md"
        jq -n --arg s "$2" --arg b "$3" '{status: $s, body: $b}' > "$TMP/fixtures/$1.json"
    }
    fixture 1238 in-progress $'## Why\n\nx\n\n## Remaining\n\n- slice 2 of 4\n'
    fixture 1239 in-progress $'## Why\n\nx\n'
    fixture 1240 in-progress $'## Why\n\nx\n'
    fixture 1241 in-progress $'## Remaining\n\n\n## References\n\n- y\n'
    fixture 1242 todo $'## Remaining\n\n- the rest\n'

    (
        cd "$TMP"
        CALLS="$TMP/stint-calls.log"
        stint() {
            echo "$*" >> "$CALLS"
            case "$1" in
                show) cat "$TMP/fixtures/$2.json" ;;
                done|unclaim) ;;
                *) echo "unexpected stint call: $*" >&2; return 1 ;;
            esac
        }
        fail() { echo "test_resolve_partial: $*" >&2; exit 1; }
        local actual

        actual=$(partial_ids_from_body "Stint-partial: 1238")
        [ "$actual" = 1238 ] || fail "expected partial id 1238, got '$actual'"
        actual=$(resolve_stints "feature/partial" "Stint-partial: 1238")
        [ -z "$actual" ] || fail "partial id must not close, got '$actual'"

        actual=$(resolve_stints "feature/stint-1238-slice" "Stint: 1238
Stint-partial: 1238")
        [ -z "$actual" ] || fail "branch and Stint: line must not override partial, got '$actual'"

        BODY="Stint: 1239
- **Stint-partial:** 1238"
        actual=$(resolve_stints "feature/partial" "$BODY")
        [ "$actual" = 1239 ] || fail "expected close set 1239, got '$actual'"
        actual=$(resolve_partial_stints "$BODY")
        [ "$actual" = 1238 ] || fail "expected partial set 1238, got '$actual'"

        : > "$CALLS"
        actual=$(do_close_stints 3300 1239 --partial 1238) || fail "close+partial failed: $actual"
        grep -qx "done 1239" "$CALLS" || fail "expected 'done 1239', calls: $(cat "$CALLS")"
        grep -qx "unclaim 1238" "$CALLS" || fail "expected 'unclaim 1238', calls: $(cat "$CALLS")"
        ! grep -qx "done 1238" "$CALLS" || fail "partial 1238 must never be done"
        [[ "$actual" == *"Stint 1239: closed"* && "$actual" == *"Stint 1238: released (partial"* ]] ||
            fail "expected closed/released lines, got '$actual'"

        # Refusals: no Remaining, empty Remaining, missing task. Nothing is done or unclaimed.
        for BAD in 1240 1241 9999; do
            : > "$CALLS"
            if resolve_partial_stints "Stint-partial: $BAD" >/dev/null 2>&1; then
                fail "expected preflight refusal for partial $BAD"
            fi
            if do_close_stints 3300 1239 --partial "$BAD" >/dev/null 2>&1; then
                fail "expected close-stints refusal for partial $BAD"
            fi
            ! grep -Eq '^(done|unclaim) ' "$CALLS" || fail "refused partial $BAD still ran: $(cat "$CALLS")"
        done

        : > "$CALLS"
        actual=$(do_close_stints 3300 --partial 1242) || fail "already-todo partial must not error: $actual"
        [[ "$actual" == *"Stint 1242: already todo"* ]] || fail "expected already-todo report, got '$actual'"
        ! grep -q '^unclaim' "$CALLS" || fail "already-todo partial must not unclaim"

        # Full flow: a partial with no Remaining refuses before the remote squash.
        actual=$(
            exec 2>&1
            gh() {
                case "$1 $2" in
                    'pr view')
                        if [[ "$*" == *closingIssuesReferences* ]]; then
                            printf '\n'
                        else
                            printf '%s\n' '{"branch":"feature/x","head":"0000000000000000000000000000000000000001","base":"alpha","state":"OPEN","body":"Stint-partial: 1240"}'
                        fi
                        ;;
                    *) echo "mock gh $*" ;;
                esac
            }
            git() { echo "mock git $*"; }
            do_sync() { echo "mock sync"; }
            set -- 3300
            eval "$(sed -n '/^# --- Dispatch ---/,$p' "$SCRIPT_DIR/merge-pr.sh")"
        ) && fail "full flow must refuse a partial without Remaining, got '$actual'"
        [[ "$actual" != *"mock gh pr merge"* && "$actual" != *"mock sync"* ]] ||
            fail "refusal must precede sync and squash, got '$actual'"
        [[ "$actual" == *"'## Remaining'"* ]] || fail "expected Remaining refusal, got '$actual'"
    ) || return 1

    echo "resolve_partial tests passed"
}

# --- Dispatch ---
CMD="${1:-}"
case "$CMD" in
    test-resolve-issue) test_resolve_issue ;;
    test-resolve-stints) test_resolve_stints ;;
    test-resolve-both) test_resolve_both ;;
    test-resolve-partial) test_resolve_partial ;;
    sync)     do_sync ;;
    cleanup)  do_cleanup "${2:?PR required}" "${3:?BRANCH required}" "${4:?lane head SHA required}" ;;
    close)    do_close "${2:?ISSUE required}" "${3:?PR required}" ;;
    close-stints)
        PR="${2:?PR required}"
        shift 2
        do_close_stints "$PR" "$@"
        ;;
    *)
        PR="${1:?PR number required}"
        if [[ ! "$PR" =~ ^[0-9]+$ ]]; then
            echo "ERROR: expected a numeric PR number." >&2
            exit 1
        fi
        shift
        NO_ISSUE=0
        while [ "$#" -gt 0 ]; do
            case "$1" in
                no-issue|--no-issue) NO_ISSUE=1; shift ;;
                *) echo "ERROR: unknown argument '$1'" >&2; exit 1 ;;
            esac
        done

        INFO=$(gh pr view "$PR" --json headRefName,headRefOid,baseRefName,state,body --jq '{branch: .headRefName, head: .headRefOid, base: .baseRefName, state: .state, body: .body}')
        BRANCH=$(echo "$INFO" | jq -r .branch)
        HEAD_SHA=$(echo "$INFO" | jq -r .head)
        STATE=$(echo "$INFO" | jq -r .state)
        BASE=$(echo "$INFO" | jq -r .base)
        if [ "$STATE" != OPEN ] && [ "$STATE" != MERGED ]; then
            echo "ERROR: PR #$PR is $STATE, not open; refusing to merge." >&2
            exit 1
        fi
        if [ "$BASE" != alpha ]; then
            echo "ERROR: PR #$PR targets $BASE, not alpha; refusing to merge." >&2
            exit 1
        fi
        PR_BODY=$(echo "$INFO" | jq -r '.body // ""')
        ISSUE=""
        STINTS=""
        PARTIALS=""
        STINT_ARGS=()
        PARTIAL_ARGS=()
        if [ "$NO_ISSUE" -eq 0 ]; then
            ISSUE=$(resolve_issue "$PR" "$BRANCH" "$PR_BODY" || true)
            # Resolved independently of the issue: a PR routinely has both. A named
            # stint with no task file is a hard error (set -e), issue found or not.
            STINTS=$(resolve_stints "$BRANCH" "$PR_BODY")
            # Preflight: a partial stint without '## Remaining' refuses before the squash.
            PARTIALS=$(resolve_partial_stints "$PR_BODY") || exit 1
            if [ -z "$ISSUE" ] && [ -z "$STINTS" ] && [ -z "$PARTIALS" ]; then
                echo "ERROR: could not resolve GitHub issue or stint tasks for PR #$PR ($BRANCH)" >&2
                echo "Link a GitHub issue through closingIssuesReferences or a closing keyword like 'Closes #1234'; for stints, use a stint-<id> branch or a 'Stint: 0123, 0124' line in the PR body ('Stint-partial: 0123' for a slice that leaves the stint open)." >&2
                echo "For a standalone follow-up PR with nothing to close, re-run: just merge-pr $PR no-issue" >&2
                exit 1
            fi
            while IFS= read -r STINT; do
                if [ -n "$STINT" ]; then
                    STINT_ARGS+=("$STINT")
                fi
            done <<< "$STINTS"
            while IFS= read -r STINT; do
                if [ -n "$STINT" ]; then
                    PARTIAL_ARGS+=(--partial "$STINT")
                fi
            done <<< "$PARTIALS"
        fi

        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> PR #$PR: $BRANCH (standalone — no issue/stint to close, state: $STATE)"
        else
            echo "==> PR #$PR: $BRANCH ($(describe_closures "$ISSUE" "${STINT_ARGS[@]+"${STINT_ARGS[@]}"}"), state: $STATE)"
            [ -z "$PARTIALS" ] || echo "==> Partial stint(s), released not closed: $(join_by ", " $PARTIALS)"
        fi

        # Advance the canonical checkout before touching the remote PR. Git refuses
        # overwriting local edits or dropping local commits.
        do_sync
        assert_clean_lane "$BRANCH"
        if [ "$STATE" != "MERGED" ]; then
            do_merge "$PR"
        else
            echo "==> PR already merged — skipping squash; resuming sync and cleanup"
        fi

        do_sync
        do_cleanup "$PR" "$BRANCH" "$HEAD_SHA"
        if [ "$NO_ISSUE" -eq 1 ]; then
            echo "==> Skipping issue/stint close (no-issue)"
        else
            if [ -n "$ISSUE" ]; then
                do_close "$ISSUE" "$PR"
            fi
            if [ "${#STINT_ARGS[@]}" -gt 0 ] || [ "${#PARTIAL_ARGS[@]}" -gt 0 ]; then
                do_close_stints "$PR" "${STINT_ARGS[@]+"${STINT_ARGS[@]}"}" "${PARTIAL_ARGS[@]+"${PARTIAL_ARGS[@]}"}"
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

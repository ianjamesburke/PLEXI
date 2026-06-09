---
name: cancel-pr
description: Use when a PR attempt has failed and needs to be abandoned — close without merging, audit what went wrong, rewrite the issue so the next agent starts closer to done with fewer wrong turns.
risk: medium
source: local
date_added: "2026-05-09"
---

# Cancel PR

Abort a failed PR attempt, extract learnings, and rewrite the issue so the next run is faster.

---

## Phase 1 — Close the PR

```bash
gh pr close <N> --comment "Closing — <one sentence: what failed and why>. Rewriting issue with learnings."
```

Do NOT delete the branch yet. The diff is evidence.

---

## Phase 2 — Foot-Gun Audit

Pull the full diff and the conversation context. Answer each question:

**Implementation mistakes:**
- What did the agent implement that was already handled elsewhere?
- What assumption did it make that turned out to be false?
- Where did it patch instead of fix root cause?
- What did it invent that should have been looked up first?

**Navigation mistakes:**
- Which files did the agent read that were irrelevant?
- Which files did it miss that it needed?
- Where did it make a decision that required backtracking?

**Spec mistakes (what the issue failed to say):**
- What constraint was discovered mid-implementation that should have been stated upfront?
- What existing code shape was unknown to the agent that caused wrong assumptions?
- What "obvious" behavior was actually non-obvious once you saw the code?

Capture each finding as a concrete bullet. Vague findings ("agent was confused") are useless. Name the specific file, wrong assumption, or missing constraint.

---

## Phase 3 — Rewrite the Issue

**Rule: rewrite the body, never append.** An issue body that accumulates addendums forces the next agent to reconcile contradictions. A clean rewrite is a fresh spec.

Fetch the current body:
```bash
gh issue view <N> --json title,body,comments --jq '{title:.title, body:.body}'
```

Rewrite with this structure:

```markdown
## Goal
[One sentence: what done looks like]

## Context
[Relevant existing code shape — file paths, key types/structs, current behavior]
[Include anything the failed attempt had to discover mid-run]

## Constraints
[Hard constraints discovered or confirmed: what must not change, what already exists, what the code shape forces]

## Acceptance Criteria
[Concrete, checkable. "cargo test passes" is not enough — name the behavior.]

## Prior Attempts

**Attempt N:** [What was tried — specific approach, not vague]
**Why it failed:** [Root cause — the actual constraint or wrong assumption]
**What to try next:** [Specific next direction, if known]
```

Write the rewritten body:
```bash
gh issue edit <N> --body "$(cat <<'EOF'
<rewritten body>
EOF
)"
```

---

## Phase 4 — Cleanup

```bash
# Re-label: remove in-progress, add ready
gh issue edit <N> --remove-label "in progress" --add-label "ready"

# Delete the remote branch
git push origin --delete <branch-name>

# Remove the worktree
wtp remove <branch-name>
```

Confirm the worktree is gone:
```bash
git worktree list
```

---

## Quality Bar for the Rewrite

The rewrite succeeds if a fresh agent reading only the issue (no session context) would:
1. Know which files to read first
2. Know what shape the existing code has
3. Know what constraints cannot be violated
4. Know what the last attempt tried and why it failed

If any of those four are missing, the rewrite is incomplete.

---

## What Not to Do

- Don't append a "## What we tried" section — integrate the learning into Constraints and Prior Attempts
- Don't preserve vague acceptance criteria hoping the next agent figures it out
- Don't delete the branch before Phase 2 — the diff is your primary evidence
- Don't re-label as `ready` if the root cause is still unknown — label `blocked` with a blocking note instead

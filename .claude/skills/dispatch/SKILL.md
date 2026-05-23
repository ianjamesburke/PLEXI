---
name: dispatch
description: Use when the user says /dispatch <issue> or wants to ship a specific issue. Requires an issue number. Spawns a background Claude sub-agent that runs ship-issue end-to-end — implement → open-pr → validate → merge. Orchestrator is free immediately after dispatch.
---

# Dispatch

Spawn a background sub-agent to ship a specific issue. The sub-agent owns the full pipeline and its own watcher loop.

## Invocation

```
/dispatch 1671        # ship issue #1671
/dispatch 1671 1679   # ship two issues in parallel sub-agents
```

An issue number is always required. Dispatch does not auto-pick.

---

## Step 1 — Alpha gate

```bash
git status --porcelain
git log origin/alpha..HEAD --oneline
```

If either has output: print `ALPHA BLOCKED — working tree is dirty or has unpushed commits.` and stop.

---

## Step 2 — Validate issue(s)

For each issue number, confirm it's open and not already in progress:

```bash
gh issue view <N> --json number,title,state,labels \
  --jq '{number, title, state, labels: [.labels[].name]}'
```

If state is `CLOSED`: stop, tell the user.
If it has `in progress` label: stop, tell the user it's already being worked.

Print what's about to be dispatched:
```
Dispatching #1671 — fix(infra/skills): implement-issue preflight optimization
```

---

## Step 3 — Spawn sub-agent(s)

For each issue, spawn a background Claude sub-agent via the Agent tool with `run_in_background: true`:

**Prompt template:**
```
You are shipping issue #<N> in the PLEXI repo at /Users/ianburke/Documents/GitHub/PLEXI.

Run the /ship-issue skill for issue #<N>. Follow it exactly — it will guide you through implement-issue → open-pr → validate-pr → merge-pr in sequence. You own the full pipeline and the watcher loop between each phase.
```

Multiple issues = multiple Agent calls in the same message (parallel).

---

## Notes

- Dispatch is fire-and-forget. The sub-agent is the watcher — do not poll it.
- You will be notified when each sub-agent completes.
- If a sub-agent fails mid-pipeline, re-dispatch the same issue — ship-issue resumes from where it left off.

---
name: wrap-up
description: "End a session at a clean stopping point and leave a one-shot RESUME.md handoff at the repo root for the next session to read and delete. Use when Ian says wrap up, stop here, or pause."
---

# Wrap Up

`RESUME.md` is a one-shot baton between sessions, not a tracker. The next session reads it, acts on it, and deletes it (rule: `AGENTS.md` → Session Resume). Durable facts never live only here: task state goes to stint, lessons to memory or a `## Traps` entry, PR state to the PR.

## Step 1: Reach a stopping point

Finish the current atomic step; start nothing new.

- No half-applied edits. Commit coherent slices on feature branches; leave unfinishable work as a `WIP:` commit. Never stash.
- Stop any test host you started (`plexi-<channel> host stop`, confirm `host status` says not running).
- Background builds/installs: let them finish if under a few minutes, else note them as interrupted.
- Sub-agents die with the session. Ask each for a one-line status, then record where its worktree and branch stand.
- Durable findings go to their owning home now (stint task body, memory, `## Traps`), not only into RESUME.md.

## Step 2: Write RESUME.md

Path is the main checkout root, shared by every worktree:

```bash
ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
```

If `$ROOT/RESUME.md` already exists, another session left it: merge, never overwrite.

Contents, terse, only what the next session cannot recover from git, stint, or PRs:

- **Written:** UTC timestamp, branch, one-line goal of the session.
- **Waiting on Ian:** decisions pending, each with the recommended call.
- **In flight:** each open PR / worktree / agent: state, last verified head sha, the exact next action.
- **Next step:** the single thing to do first.
- **Do not:** traps hit this session that are not yet written down elsewhere, or constraints Ian set that apply to the in-flight work.

Pointers over prose: `stint 0596 (install integrity) ## Live Finding`, not a restatement.

## Step 3: Report

Open it for Ian: `"$ROOT/wrap-up.sh"`. Reply with one line confirming the stopping point and anything left running.

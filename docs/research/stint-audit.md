# Stint audit

Date: 2026-10-05. Research note. This is not a PRM and it does not change the host.

The ledger under test is the external crate [`ianjamesburke/stint`](https://github.com/ianjamesburke/stint) `0.3.17`, commit `d1aadc7665f0ae5ddb2d204b97a340181fc73639` (2026-09-13). PLEXI does not vendor it and does not list it in `Cargo.toml`. The binary exercised here is a `--locked` release build of that commit (`stint 0.3.17`).

Scripts: `scripts/stint-stress/`. Full suite transcript: `scripts/stint-stress/results/run-all.txt` (started `2026-10-05T22:28:19Z`). The lock-wait and 10k timings were re-run after the scripts stopped calling a missing `/usr/bin/time`; that transcript is `scripts/stint-stress/results/rerun-03-05.txt` (started `2026-10-05T22:29:16Z`). Host: Linux 6.12.94+ x86_64.

## 1. What stint is

Stint is a per-repo task ledger stored as one markdown file per task, plus optional sprint index files. A CLI and a TUI read and write that directory. PLEXI agents use the CLI as the operating graph for implementation work.

### Data model

A task file is YAML frontmatter between `---` delimiters, then a free markdown body. `serialize_task` writes the fields below. `RawFrontmatter` in `parse.rs` also accepts a `sprint` key and then discards it; sprint membership lives only in the sprint index.

| Field | Role |
|---|---|
| `id` | Decimal string, zero-padded to at least four digits (`"0001"`). Parsed as text. |
| `title` | Display title. The filename slug is derived from it at create time. |
| `status` | `backlog`, `todo`, `in-progress`, `done`, `archived`. |
| `priority` | Optional `p0`–`p4`. `p0` sorts first. Missing priority sorts last. |
| `size` | Optional `s`, `m`, `l`. |
| `estimate`, `actual` | Duration strings (`4h`, `30m`). `stint log` accumulates `actual`. |
| `created_at`, `started_at`, `completed_at` | UTC RFC3339 strings. `started_at` is the only claim marker. |
| `blocked_by` | Mixed list. Syntax selects the variant: bare integer (local task), `@N` (local GitHub issue), `../path:NNNN`, `../path@N`, `owner/repo:NNNN`, `owner/repo@N`, a direct `*.md` path under `.stint/tasks/`, or any other string as a free-text note. |
| `gh_issue`, `area`, `tags` | String lists. `area` is also a scheduling mutex for `stint next` and auto-claim. |

There is no owner, agent id, run id, claim generation, client tag, or work-kind field. Two successful claims of the same task are indistinguishable in the file: the last write leaves one `started_at`.

`classify` in `state.rs` maps each task to one of `backlog`, `ready`, `blocked`, `active`, `done`, `archived`. `todo` with an unresolved blocker is `blocked`. `in-progress` is `active` even when blockers remain; `stint check` is what flags that as `BlockedTaskNotPending`. Local blockers whose task is `done` drop out of the active set. Issue refs, external refs, and free-text notes stay active forever, because stint never resolves them.

`compute_next` keeps `todo` tasks (backlog only with `--include-backlog`), drops unresolved blockers, then drops tasks whose `area` overlaps an in-progress task or an earlier ready task in the same pass. Sort is priority, then `created_at`, then the id **string**.

### Storage

```
.stint/
  config.toml          # project name, default_sprint, gh.repo
  tasks/NNNN-slug.md   # one file per task
  sprints/sN.md        # ordered markdown links
  add.lock             # flock file for `stint add` (kernel-released on death)
  claim.lock           # directory created by auto-claim only
```

`StintRepo::find` walks parents until it sees `.stint/`. `write_task` is `std::fs::write`: create/truncate, then write, no temp file, no fsync, no rename. `load_tasks` prints `warning: skip …` and omits any file that fails to parse. `stint check` uses `load_tasks_with_errors` and exits 1. `stint list` and `stint next` use the skipping loader and can exit 0 with the bad file gone.

Ids come from `next_task_id`: max numeric id plus one, formatted `{:04}`. `cmd_add` computes that inside `with_add_lock`, an exclusive `flock` on `add.lock`. The kernel drops that lock if the process dies.

### Commands

`stint --help` on this build:

```
init, add, list, show, edit, set, remove,
claim, unclaim, done, log, archive, next,
ready, defer, sprint, check, status, update, completions
```

`claim` help says: "Mark a task in-progress. Without an ID, claims the top ready task (lock-protected)." The parenthetical applies only to the no-id path. `stint claim <id>` does not take `claim.lock`.

| Command | What it writes |
|---|---|
| `add` | Next id under `add.lock`, status `todo` (or `backlog` with `--backlog`). |
| `claim ID` | `cmd_start`: status `in-progress`, set `started_at` if absent. No lock. |
| `claim` | Under `claim.lock`, `cmd_start` on the first row of `stint next`. |
| `claim --restart` | Replace `started_at`, clear `completed_at` and `actual`. |
| `unclaim` | `in-progress` → `todo`, clear `started_at`. Any caller. |
| `done` | status `done`, set `completed_at`, optional `actual`. |
| `next` | Read-only ready/blocked report. |
| `check` | Parse errors, duplicate ids, cycles, bad enums, in-progress tasks that are still blocked. |
| `update` | `cargo install stint --locked --force`. |

`with_claim_lock` is `mkdir .stint/claim.lock`, run the closure, `rmdir`. On `AlreadyExists` it sleeps 50ms and retries. The 21st failure returns `claim lock held after 21 retries; remove .stint/claim.lock to reset`. A kill between `mkdir` and `rmdir` leaves the directory. Nothing records a pid, and nothing steals a dead holder's lock. `stint --help` has no recover or reap command.

`cmd_start` does not look at `status`. It refuses a second call only when `started_at` is already set, unless `--restart`.

### Claim semantics

1. **Named claim** (`stint claim 0001`, the call PLEXI skills make) is a read-modify-write of one file with no lock and no owner.
2. **Auto-claim** (`stint claim`) holds the directory lock across `load_tasks` plus `cmd_start`, so concurrent auto-claimers each receive a different ready task, until the lock is stale or the 1 second budget runs out.
3. A second claim by the same caller is an error, not a replay of the first claim. `--restart` is a new timestamp.
4. `started_at` is not a lease. A years-old `in-progress` task stays active. `stint next` will not offer it. The same area stays busy for auto-claim.
5. `unclaim` is the inverse, and it does not check who claimed.

### Where PLEXI uses it

PLEXI treats `.stint/` as the live work graph and gitignores `.stint/*` (`.gitignore`). The host never opens those files. Agents shell out to the CLI.

| Caller | Commands |
|---|---|
| Root `AGENTS.md` | `stint claim` when work begins, `stint done` when it ends. No `stint start`. `stint check`, `list`, `show`, `status`, `next`. |
| `.claude/skills/create-stint/SKILL.md` | `stint add`, `list`, `check`. Flat list: priority plus `blocked_by`, no sprints. |
| `.claude/skills/implement-issue/SKILL.md`, `implement-stint-v2`, `disabled.implement-stint` | `stint claim <task-id>` on the alpha checkout before the worktree. `stint next` when no id is given. |
| `.claude/skills/merge-pr/SKILL.md` | `stint done <task-id>` after a successful merge. |
| `.claude/skills/validate-pr/SKILL.md` | `stint show` as the brief when the branch is stint-first. |
| `docs/agent-run-orchestration.md` | `stint show` is world state the supervisor reconciles against. The failure note still says ids are `ls \| sort \| tail` with no lock, and that `stint claim` is already lock-protected. |
| `docs/specs/agents-api-and-permission-gate.md` | P4 "Tasks / links": an assignment projection plus external stores, and "No duplicate stint completion state." |

That orchestration paragraph is stale against `0.3.17`. `cmd_add` already flocks. The concurrent-add case below got 16 unique ids. The claim half of the sentence is wrong for the command PLEXI actually runs: `stint claim <task-id>` never takes `claim.lock`.

`docs/2026-08-01-stint-audit-and-sequencing.md` is an audit of the task graph, not of this CLI.

Because `.stint/` is gitignored, PLEXI worktrees do not share the ledger through git. `StintRepo::find` walks parents, so a nested worktree can silently attach to a parent checkout's ledger, and a sibling worktree starts empty. Claim state is local to whichever directory the walk hits. There is no host lease tying a claim to a pane, a run, or a machine.

## 2. Stress results

Every case uses a temporary repo (`stint init`) and the release binary above. Re-run with `scripts/stint-stress/run-all.sh`. `STINT_BIN` overrides the binary; otherwise the script clones `STINT_REV` and builds it.

### 2.1 N processes claim one task

`01-concurrent-same-task.sh`. One `todo` task with a 2MiB body, so the truncate-then-write window is wide enough to hit. N=32 processes spin on a barrier, then run `stint claim 0001 --json`. Five rounds.

| Round | exit 0 | `already has started_at` | `missing frontmatter` |
|---|---|---|---|
| 1 | 8 | 21 | 3 |
| 2 | 8 | 1 | 23 |
| 3 | 8 | 21 | 3 |
| 4 | 9 | 16 | 7 |
| 5 | 8 | 24 | 0 |

`RESULT explicit_claim any_round_with_multiple_successes=1`

Round 2 is the torn-read case. Eight processes printed a success document:

```json
{
  "claimed": "0001",
  "path": "/tmp/stint-stress.CUVWqN/.stint/tasks/0001-contended.md"
}
```

Twenty-three others failed while the file was truncated:

```
error: parse .../0001-contended.md: missing frontmatter opening delimiter '---'
```

One failed with:

```
error: task 0001 already has started_at; use --restart to replace it
```

After each round the file was valid again (`status: in-progress`, one `started_at`). The file does not record that eight or nine processes were told they won. There is no `assignment_conflict` error and no holder id.

**Exactly one process does not win.**

Control, same script: eight processes run `stint claim --json` (no id) against eight area-free tasks. The directory lock serializes them.

```
successes=8 failures=0
distinct=8 nonempty=8
```

Worker claims were `0001` through `0008`, each once.

Control: 16 concurrent `stint add` calls. `add.lock` held. 16 files, ids `0001`–`0016`, no duplicates. The create path is safe on one machine. The named-claim path is not.

### 2.2 Same claimer retries

`02-retry-same-claimer.sh`.

First `stint claim 0001 --json` exits 0 and sets `started_at: "2026-10-05T22:28:22Z"`. The second call, same id, no `--restart`:

```
error: task 0001 already has started_at; use --restart to replace it
exit=1
started_at_unchanged=yes
```

The JSON body is not returned again. `--restart` exits 0 and moves `started_at` to `2026-10-05T22:28:23Z`. That is a new claim, not a replay.

`unclaim` then `claim` also mints a new timestamp (`22:28:23Z`, then `22:28:24Z`).

Explicit claim does not consult the scheduler:

- A task `blocked_by` task 0001 is absent from `stint next` (`Ready (1)` lists only the blocker). `stint claim 0002` exits 0. `stint check` then exits 1: `status 'in-progress' but blocked by unresolved task(s) ["0001"]`.
- `stint done 0001 --actual 1h` on a never-claimed task leaves `status: done`, no `started_at`, and a `completed_at`. `stint claim 0001` exits 0 and sets `in-progress` while **leaving `completed_at` in place**.
- `stint archive` then `stint claim` moves `archived` back to `in-progress`.
- `--backlog` hides the task from `stint next` (`Ready (none)`). `stint claim 0001` still exits 0 and marks it `in-progress`.

**Retry is not idempotent.** It errors, or `--restart` overwrites the only claim marker. Claim also reopens terminal and iced tasks.

### 2.3 Crash mid-claim and stale recovery

`03-crash-and-stale-lock.sh`, timings from the rerun.

**Dead directory lock.** `mkdir .stint/claim.lock` with no holder. Auto-claim:

```
error: claim lock held after 21 retries; remove .stint/claim.lock to reset
elapsed_sec 1.004 max_rss_kb 11648
exit=1
lock_still_there=yes
```

The same lock does not cover a named claim. `stint claim 0001 --json` exits 0 in 0.002s and writes `in-progress`.

**Kill during auto-claim.** 400 task files. `strace -e inject=read:delay_enter=2000` slows reads after the lock is taken. The watcher sends `kill -9` to the process group once `.stint/claim.lock` exists.

```
saw_lock_before_kill=yes
lock_remains=yes
in_progress_files=0
```

The next auto-claim hits the dead lock and exits 1 after 1.003s with the same "remove .stint/claim.lock" error. No task was claimed. `rmdir .stint/claim.lock` then `stint claim --json` exits 0 and claims `0001`. Recovery is an operator deleting a directory. `stint --help` has no recovery verb. The error text is the runbook.

**Kill during the named-claim write.** `strace -e inject=write:delay_enter=3000000` stalls the first `write` after `File::create` has truncated the file. `kill -9` lands while the size is 0.

```
bytes_before=192
saw_empty_file_before_kill=yes
bytes_after=0
```

```
$ stint check
.../0001-torn-write.md: missing frontmatter opening delimiter '---'
exit=1

$ stint list
warning: skip .../0001-torn-write.md: missing frontmatter opening delimiter '---'
(no tasks)
exit=0

$ stint claim 0001 --json
error: parse .../0001-torn-write.md: missing frontmatter opening delimiter '---'
exit=1
```

The task is gone from `list` and cannot be claimed again. `check` is the only command that fails closed.

**No time-based reclaim.** `stint claim 0001 --started-at 2020-01-01T00:00:00Z` on a task with `area: host/pane`, plus a second task in that area. `stint next` prints `Ready (none)`. A second `stint claim 0001` errors on `started_at`. A different process then runs `stint unclaim 0001` (exit 0) and `stint claim 0001` (exit 0). Auto-claim while `0001` is in progress returns `{"claimed": null}` and exit 0, because the shared area hides task 0002. `stint next --include-area-conflicts` shows 0002 with `area busy: 0001 in progress`.

Stale recovery is "anyone may unclaim." A dead holder who is not unclaimed blocks the task and, when areas match, the whole area.

### 2.4 Partial writes and corruption

`04-partial-and-corrupt.sh`. Healthy file: `check` prints `ok`, `list` shows `ready`, `next` offers it.

| Fixture | `check` | `list` / `next` | `show` |
|---|---|---|---|
| Empty file | exit 1, missing `---` | warning, skip, exit 0, `(no tasks)` / `Ready (none)` | exit 1 |
| Frontmatter cut off before the closing `---` | exit 1, missing `---` | skip, exit 0 | exit 1 |
| First 40 bytes of a healthy file (`---\nid: "0001"\ntitle: "healthy"\nstatus: `) | exit 1, missing `---` | skip, exit 0 | exit 1 |
| Conflict markers in the frontmatter | exit 1, yaml scan error | skip, exit 0, `Ready (none)` | exit 1 |
| `status: running` | exit 1, unknown status | skip, exit 0 | exit 1 |
| Valid frontmatter, body deleted, `status: in-progress` | `ok` | `list` shows `active`; `next` offers nothing | exit 0, no body |
| `0003-garbage.md` (`not markdown at all`) next to two healthy tasks | exit 1 naming 0003 | warning, then lists 0001 and 0002 and `next` offers both | `show 0003` exit 1 |

`list` and `next` treat a torn file as absence and still exit 0. A truncated-but-valid frontmatter is silent data loss: `check` passes. Garbage beside good tasks does not stop the scheduler; the bad id simply disappears until someone runs `check`.

### 2.5 Large ledger (10k tasks)

`05-large-ledger.sh`, rerun. 10,000 files, ids `0001`–`10000`, about 40MB, priority `p{i % 5}`, identical `created_at`. Release binary.

| Command | exit | elapsed | max RSS |
|---|---|---|---|
| `stint check` | 0 (`ok`) | 0.095s | 11756 KB |
| `stint status` | 0 (`Active: 10000`) | 0.082s | 11764 KB |
| `stint next` | 0 (`Ready (10000)`, 10001 lines) | 0.086s | 13360 KB |
| `stint list --priority p0` | 0 (2001 lines) | 0.082s | 11744 KB |
| `stint claim 5000 --json` | 0 | 0.004s | 11764 KB |
| `stint show 5000` | 0, `in-progress` | 0.117s | 11772 KB |
| `stint add "one more"` | 0, file `10001-one-more.md` | 0.074s | 11780 KB |

Ten thousand small files are cheap on this machine. The scale failure is ordering, not time. `next_task_id` / list order compare id strings. `9995` and `10000` are both `p0` with the same `created_at`. `stint list --priority p0` places `10000` at index 200 and `9995` at index 1999 (`10000_before_9995 True`). `stint next` therefore prefers id `10000` over id `9995`.

`stint claim 5000` does not load the ledger to decide exclusivity. It is a single-file write, which is why it is faster than `show` and why section 2.1 can race.

### 2.6 Git merges

`06-git-merge.sh`. There is no single tasks ledger. The record is the task file. The closest shared index is `sprints/sN.md`. PLEXI gitignores `.stint/*`, so this is the failure mode of stint's own "commit the tasks" design, not of a PLEXI push. It is still how two branches of a committed ledger combine, and how an export would combine if the host ever wrote one.

**Same task file, two branches.** `lane-a` runs `stint claim 0001`. `lane-b` rewrites the body and sets `status: done`.

```
$ git merge --no-edit lane-b
exit=1
CONFLICT (content): Merge conflict in .stint/tasks/0001-shared-task.md
```

The file contains `<<<<<<< HEAD` around `status:`. `stint check` exits 1 with a yaml scan error. `stint list` prints the warning, then `(no tasks)`, exit 0. `stint show 0001` exits 1. The conflict deletes the task from the scheduler.

**Divergent adds.** From the same parent, each branch runs `stint add` once. Both allocate `0002` (`0002-left-only.md`, `0002-right-only.md`). The merge is clean:

```
$ git merge --no-edit add-right
exit=0
Merge made by the 'ort' strategy.
 create mode 100644 .stint/tasks/0002-right-only.md
```

`stint check` exits 1: `duplicate task id "0002" found in "0002-right-only.md" and "0002-left-only.md"`. `stint list` exits 0 and prints both rows. `stint next --json` exits 0 and returns one `0002`, titled `right only`. `ordered_tasks` inserts each id once, so the other file (`left only`) is dropped with no error. `stint show 0002` exits 1: `ambiguous id "0002": multiple matches`.

**Sprint index.** Both branches append a different link for their own `0002-*.md` onto `s1.md`.

```
$ git merge --no-edit sprint-right
exit=1
CONFLICT (content): Merge conflict in .stint/sprints/s1.md
```

```
- [0001](../tasks/0001-shared-task.md)
<<<<<<< HEAD
- [0002](../tasks/0002-left-sprint-task.md)
=======
- [0002](../tasks/0002-right-sprint-task.md)
>>>>>>> sprint-right
```

`stint sprint show s1` exits 0 and prints task 0002 twice, both titled `left sprint task`. The right-hand link is dropped. `stint check` exits 1 for the duplicate id only. Conflict markers in the sprint file are not themselves an error.

## 3. Comparison

The P2 shape in `docs/specs/agents-api-and-permission-gate.md` already has assignments (`kind: system|output`), runs, reservations, a drain record with `caused_by`, and intake dedup on authenticated owner plus request id. It does not yet say what a second claim of the same assignment returns, and it does not log a wakeup as its own record. The three Paperclip behaviors below are the gap.

| Behavior | Stint 0.3.17 | P2 agents API as written |
|---|---|---|
| Claim once. The same caller retrying gets the same claim. A competing run gets `assignment_conflict`. | Named claim has no owner. Retry exits 1 (`already has started_at`) or `--restart` changes `started_at`. Concurrent callers can all exit 0 (section 2.1). A competitor can `unclaim` and take the task. Auto-claim is exclusive only while `claim.lock` is alive. | `Assignment` has an owner agent. `Run` has `claimed_state` and a generation. "Race two requests against one remaining slot: at most one wins" is about budget admission, not about claiming one assignment. "Retrying admission reuses its reservation" is about the allocation tree. Intake dedup ("same payload returns the existing receipt; changed payload conflicts") is per request id, not per assignment claim. Nothing names `assignment_conflict` or says a retry returns the same run id. |
| Logged wakeups with a cause and a dedup key. | No wakeup log. `stint next` is a pull. A crash leaves either a directory or an empty file; the next agent notices only by failing or by seeing an empty queue. | `AgentEvent` has `caused_by`, `run_id`, `assignment_id`. Subscribers resume from a cursor. "At-least-once delivery plus dedup" is stated for drain delivery, with request id as the intake dedup key. There is no wakeup record whose cause is `blocker_cleared` or `heartbeat_timeout` and whose dedup key makes a repeated wake a no-op. |
| Client tag and system/output tag set at run start. | `tags` on the task are a free-form list, editable later with `stint set`. They are not frozen, and they do not say which client claimed the task. Nothing copies a system/output kind onto a run, because there is no run. | `Assignment.kind` is required `system\|output`. `Run` pins `config_snapshot` and a generation, and it does not carry a client tag or a frozen copy of `kind`. P4 says a later classification correction recomputes projections. The run itself does not snapshot the tags it started with. |

Section 2.1 is the concrete miss against claim-once: eight or nine of 32 processes received a success document for one task, and the retry in section 2.2 did not return that document. Section 2.3 is the miss against logged wakeups: the dead lock and the empty file are silent until a later command happens to fail, and `list` exits 0. Section 2.2's `--restart` is the miss against tags-at-start: the only "run" marker is a timestamp that the next caller is invited to replace.

P2 already requires the pieces these bugs need: one writer, torn records as visible errors (the drain section), observed liveness separate from a claim of `running`, and "No duplicate stint completion state" on the Desk projection. The stress run shows the current ledger violating each of those if it stays the authority.

## 4. Recommendation

Fold stint into the agents API as the task and assignment layer. Keep the `stint` CLI as an adapter that calls that API and passes through the host permission gate. The task board is the P4 Desk projection (headless queue plus the tasks region), not a second app and not `.stint/` on disk.

The authority for "this task is claimed, by whom, as which run" becomes `Assignment` plus `Run` in the host journal (`~/.plexi/agent-records/<host-id>/` in the P2 storage section). Markdown files, if they still exist, are an export. A process that writes a task file has not claimed anything. `stint claim` in the skills keeps its spelling and starts meaning `assignments.claim` under the caller's run token.

### Keep

- The task as the unit of work: title, markdown body, priority `p0`–`p4`, size `s`/`m`/`l`, `blocked_by` as a real graph, `area` labels, `gh_issue` as a foreign key.
- The lifecycle words `backlog`, `todo`, `in-progress`, `done`, `archived`, mapped onto assignment state. `ready` and `blocked` stay derived.
- `stint next` ordering: priority, then `created_at`, then id, with unresolved blockers excluded. Area overlap stays a scheduling hint and becomes a host lease, not a sticky mutex.
- CLI spellings the skills already call: `add`, `list`, `show`, `next`, `claim`, `unclaim`, `done`, `check`, `status`, `set`.
- `stint check` as a check that the export matches the journal.
- The `add.lock` idea: create is exclusive. The host issues opaque ids, so the flock no longer protects a `max+1` counter. The concurrent-add result (16 unique ids) is the behavior to preserve.
- A human-readable markdown export for people who want tasks in a diff.

### Fix

- Named claim and auto-claim go through one admission. The caller is an agent id on a run. The same run's retry returns the same claim. Any other run gets `assignment_conflict` and no write.
- Store owner, run id, and generation. Stop using `started_at` as a lock bit.
- Replace `mkdir claim.lock` and the 21×50ms timeout with a host lease that ends when the holder dies. Delete the "rmdir the lock" recovery.
- Commit with a temp file, fsync, and rename, or append to the journal. `list` and `next` must not skip a torn record and exit 0.
- Reject claim of `done`, `archived`, `backlog`, and blocked tasks. Do not leave `completed_at` set on an `in-progress` task.
- `unclaim` only for the holder, or as a host recovery that writes a wakeup. A second process must not be able to steal by unclaiming.
- Compare ids numerically, or stop issuing decimal `max+1` ids. `10000` sorting before `9995` is a live bug once a ledger crosses 9999.
- Duplicate ids fail `list`, `next`, and `show`, not only `check`. `next` must not drop one duplicate and exit 0.
- Conflict markers in a task file or a sprint file fail `check` and `sprint show`.
- The claim help line. It currently says the command is lock-protected.
- `docs/agent-run-orchestration.md`'s id-collision paragraph. Creates are flocked; `stint claim <id>` is not.

### Drop

- Sprints as a scheduler. PLEXI already runs a flat list. The sprint file is a second index that merges badly and parses conflict markers as success.
- The TUI as the product UI.
- `stint update` (`cargo install`) inside the adapter.
- `stint log` / `actual` as the cost model. P2 already has budget, reservation, and measured usage. Do not infer billable time from a timestamp.
- Free-text note blockers as a permanent silent block. A blocker the host cannot resolve is a typed request, or it is not a blocker.
- Cross-repo path syntax (`../plexi:0146`, `owner/repo:NNNN`) as identity. Opaque ids and explicit links replace it.
- `claim.lock` directories and zero-padded `max+1` ids as the identity of work.
- Git merge as the concurrency protocol for the live ledger. PLEXI already gitignores `.stint/*` because that protocol does not work; the replacement is the host journal, not "commit the files anyway."

### Concrete spec edits

Edit `docs/specs/agents-api-and-permission-gate.md`. Proposed text follows the existing "proposed, not implemented" rule.

**P2 records.** Extend `Run` and add the claim and wakeup types:

```rust
struct Run {
    id: RunId, agent: AgentId, assignment: AssignmentId,
    delegation: Option<DelegationId>, parent_run: Option<RunId>,
    config_snapshot: ConfigRef, generation: u64, deadline: Timestamp,
    observation: LivenessObservation, claimed_state: Option<Claim>,
    reservation: ReservationId, result: Option<TypedResultRef>,
    tags: RunTags, // frozen at create; later corrections are new events
}
struct RunTags {
    client: ClientName,    // "stint" | "desk" | "mcp" | "phone" | "assistant"
    work_kind: WorkKind,   // snapshot of Assignment.kind at run start
}
struct AssignmentClaim {
    assignment: AssignmentId, run: RunId, owner: AgentId,
    generation: u64, claimed_at: Timestamp, lease_until: Timestamp,
}
struct Wakeup {
    record_id: RecordId, run: Option<RunId>,
    assignment: Option<AssignmentId>, cause: WakeCause,
    dedup_key: String, at: Timestamp,
}
enum WakeCause {
    AssignmentClaimed, BlockerCleared, RequestResolved,
    Timer, HeartbeatTimeout, LeaseRecovered,
}
```

`dedup_key` is the authenticated owner, the cause, and the source record id. A second append with the same key returns the existing wakeup and does not start a second run.

**P2 operations table.** Add a row:

| `assignments.claim` | If the assignment is claimable and unclaimed, persist one `AssignmentClaim` and one `Run` whose `tags` are set in that write, then return them. If the same owner retries the open run, return that claim unchanged (same run id, same generation). If a different run claims it, return `assignment_conflict` with the holder run id and generation, and do not write. `done`, `archived`, `backlog`, and blocked assignments return `not_claimable`. | `stint claim ID` / `stint claim`; `plexi agent assign claim ID` |

Add `assignments.unclaim` as holder-only, or host recovery that appends `WakeCause::LeaseRecovered` before the assignment is claimable again.

**P2 prose, after the delegation section.** A claim is a lease plus a generation, not a timestamp. Host observation decides staleness. A claim of `running` still cannot keep a dead worker green (existing sentence). When the lease expires or the holder dies, the host appends one `LeaseRecovered` wakeup and only then allows a new claim. Two `assignments.claim` calls in the same moment produce one winner and one `assignment_conflict`. A retry after a lost response returns the winner's claim when the caller is that winner, and `assignment_conflict` otherwise. Adapters do not mkdir a lock directory and do not truncate a record in place. A torn claim is a visible error on list and on next; those calls do not skip the record and exit 0.

**Drain.** Wakeups are `AgentEvent`s with `event_type = "wakeup"`. `caused_by` still points at the source record. The wakeup payload carries `cause` and `dedup_key`.

**Stint adapter.** Add a short subsection next to the MCP adapter:

The `stint` CLI is an adapter. `stint add` calls `assign` and sets `kind` from a flag (default `output`). `stint claim` and `stint claim ID` both call `assignments.claim` and send `client = "stint"`. `stint next` queries claimable assignments. `stint done` completes the run. `stint check` compares an optional markdown export to the journal. The permission gate authorizes claim, complete, and export. `.stint/tasks/*.md` is not the store. Skills that run `stint claim <id>` keep that spelling.

**P4 tasks row.** Replace the current "Tasks / links" cell with:

Assignment projection is the task board. Workspace and terminal links are references. A stint markdown tree, when present, is an export of these records. Completion state lives on the assignment and its run. A separate stint app does not store a second status.

**Acceptance criteria.** Add:

8. **AT-P2-08 — Claim once.** Rust tests `claim_retry_returns_same_run` and `competing_claim_returns_assignment_conflict`. Thirty-two concurrent claims of one assignment produce one success and thirty-one `assignment_conflict` results. The winner's retry returns the same run id and generation. A claim of a done, archived, backlog, or blocked assignment returns `not_claimable` and does not write. Crash before the claim record is durable leaves the assignment unclaimed and visible. Crash after it is durable leaves one generation; the winner's retry does not mint a second run.
9. **AT-P2-09 — Wakeup dedup.** Rust test `wakeup_dedup_key_is_stable`. A blocker clearing, a heartbeat timeout, and a lease recovery each append one wakeup. Replaying the same cause and source record returns the existing record id. `list`/`next` equivalents do not drop a torn tail and exit 0 (extends the torn-record rule already in AT-P2-04).
10. **AT-P2-10 — Run tags at start.** Rust test `run_tags_frozen_at_start`. The run's `client` and `work_kind` are written in the create of the run. A later correction of the assignment kind appends an event and recomputes projections. It does not rewrite the run's tags.

### A standalone stint app

A standalone stint Plexi app is not worth building. It would be a second writer of completion state, which P4 already rules out, and it would wrap the file ledger these tests just showed losing claims, hiding torn files, and merging duplicate ids. The UI is the Desk task board over the assignment projection: the headless queue for runs, and the tasks region for the ledger. If that board needs to ship before the rest of Desk, it is still a view of the same records, with `stint` as the CLI adapter in front of the permission gate.

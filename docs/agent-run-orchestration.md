# Agent Run Orchestration

Status: active

Stint: 0628, 0664–0673

The babysitter loop lands stint queues by spawning worker and tester panes, routing verdicts between them, and merging. Its contract lives entirely in `.agents/skills/babysitter/SKILL.md` — prose, executed inside a model's context window. This PRM defines the destination: the deterministic half of that loop becomes host-owned code and host-owned state, the judgment half stays with a model, and the live-host half stays in a pane. It is the spec for what Plexi must provide, not a rewrite plan for the skill.

## Call

**The thesis — "the babysitter head is a workflow engine implemented in prose" — is correct in direction and wrong in sequencing.**

Correct, because every failure this loop has produced live is a control-plane failure, not a reasoning failure: a head wedged 3h10m while its own status slot claimed `running`; ~17 lanes each running `cargo build --release` into OOM; stint id collisions; a merge runner correctly refusing a conflict with nowhere to route it. Not one of those was a model reasoning badly about code. They were spawn, wait, admission, liveness, and state — control flow. Control flow written in prose has no type system, no exit codes, and no memory that outlives a context window, which is exactly why `RUN_STATE.md`, the relay-to-a-fresh-pane ritual, the failsafe heartbeat, and the hand-built concurrency caps all exist. Every one of those is a workaround for a missing primitive.

Wrong in sequencing, because **porting the loop to code today buys almost nothing.** The host still has no push completion channel, so a Rust or TypeScript control loop would poll `plexi pane slot read` on a timer exactly like the prose head does. It would wedge in a nicer language. The primitives come first; the engine is what you build *on* them, and it is the smaller half of the work.

Wrong also in its implied destination. The reference model — subagents with pushed completion, addressable resume, `isolation: "worktree"` from one flag, automatic concurrency caps, a workflow layer where `pipeline()` and `parallel()` are real functions — is the right set of contracts and the wrong place to put them. Copying it as a babysitter-internal engine would build a private orchestrator inside a repo whose north star is that *every* capability is reachable through the Plexi CLI and that agents and terminals share one inheritable place. **These are Plexi product surface.** A host that can supervise, observe, and resource-govern a fleet of agent panes is the thing Plexi is trying to be. The babysitter is its first consumer, not its owner.

So: **build the primitives into the host, move the run's state out of a model's context and onto disk in an open format, and let the prose skill keep driving until each primitive has replaced the workaround it obsoletes.** The skill shrinks to the judgment it was always the only viable home for.

---

## The split

This is the whole spec. Three categories, and the boundary between the first two is the one that matters.

### Deterministic — belongs in code

No taste is involved. Every one of these has a right answer derivable from state, and every one of them is currently prose:

- **Queue resolution and sprint re-resolution.** `stint sprint show` is already deterministic; re-resolving after every merge to catch newly-unblocked tasks is a fold, not a judgment.
- **Lane admission.** Concurrency caps, the global one-cargo-build lease, pause-on-cap. Currently a number in `RUN_CONFIG.toml` that a head must remember to honor.
- **Spawn and boot confirmation.** Create pane, wait for a booted idle prompt, fail with a typed error on timeout, close the orphan.
- **Waiting for a terminal state.** The single highest-value conversion: an event, not a loop.
- **Liveness.** Whether a lane is alive is an observation about a process, never a claim written by that process.
- **Retry and respawn** on engine wall / boot failure, including the codex→claude family fallback.
- **Artifact verification.** `gh pr checks`, `MERGEABLE`, installed-build head-sha match against `gh pr view --json headRefOid`, `stint show` after merge. All currently prose instructions to "verify, don't trust the pane's self-report" — which is code's job, not a discipline to remember.
- **Clean-merge dispatch and its outcome check.**
- **Run record write, log append, resume.**
- **Stand-down.** Cap pause, machine-pressure throttle, scratch reaping, closing panes with nothing to protect.

### Model judgment — genuinely needs a model

None of these have a right answer derivable from state. They are the reason a model is in this loop at all:

- **Batching.** Which stints combine into one PR is taste: shared subsystem, cohesion, whether a split PR could even be CI-green on its own. A dependent-chain-lands-as-one-PR call is judgment about meaning, not about file overlap.
- **Tier selection** per batch, and mid-run escalation when a lane is fumbling.
- **Brief authoring** — the overlay of run-specific facts, decisions not yet in task bodies, cross-batch gotchas.
- **Reading a verdict.** Is this bug list real, a near-pass whose gate merely did not complete, or a false FAIL the head itself caused with a badly-worded criterion? Overruling a FAIL on the record is judgment and must stay judgment.
- **Regression vs pre-existing**, and whether a proven-pre-existing finding drops from this PR's gate into a follow-up.
- **Rebase conflict resolution.**
- **Escalation and parking.** Deciding a question genuinely needs Ian.

### Live host — genuinely needs a pane

A pane is a terminal attached to a running host. These need that and nothing else does:

- `just pr-install <N>` and booting `plexi-pr-<N>` — long-running, produces a real installed channel profile.
- **Driving the installed app end to end**, including falsifiability probes (break the invariant, watch the guard fire, revert).
- **Scene capture** and framebuffer shots via `drive-host`.
- Anything that must observe `~/.plexi-pr-<N>/plexi.log` from the running build.

**The load-bearing correction:** *worker implementation is not on this list.* A worker needs a worktree, a long-running subprocess, and a model — it never needs a terminal a human is watching. It runs in a pane today only because a pane is the sole agent-spawn primitive Plexi has. That is a misuse of a human-trust affordance as a process supervisor, and it is the same category error `AGENTS.md` already names when it forbids app-to-app data over PTY injection. Testers need a host. Workers need a supervisor. Conflating them is why a worker's result has to be scraped off a screen.

---

## What the host must provide

Each of these is a stint. They are ordered by how much workaround they delete.

### 1. Pane lifecycle events — the single biggest gap

Everything polls because nothing pushes. Panes must publish lifecycle events on the **existing event bus** (`DeclareEventStreams` / `EmitEvent` / `SubscribeAppEvents`) rather than through a new transport — `AGENTS.md` already lists pane slots as a consolidation candidate for exactly this bus, and a second event mechanism would violate the one-comms-model rule.

Event kinds, at minimum: pane spawned; agent booted to an idle prompt; agent went idle; agent blocked, carrying a typed reason (`permission-prompt`, `usage-limit`, `boot-failure`); slot changed, carrying name and value; pane exited with its status.

The consumer surface is a blocking wait that the *host* resolves, so a client burns no tokens and no wakeups while waiting:

- `plexi pane wait <id> --until <predicate> --timeout <s>` — blocks in the host, exits 0 with the matched event, exit 2 on timeout, exit 1 on plumbing. This generalizes `pane slot wait`, which already proves the shape.
- `plexi pane events --follow [--pane <id>]` — a stream for a supervisor watching many lanes at once.

A workflow awaits this the same way it awaits any other future: one call per lane, resolved by the host in ~event latency rather than in `pane_event_poll_seconds`.

### 2. Observed liveness, separate from claimed state

The 3h10m wedge is a two-field problem collapsed into one field. A slot is a **claim** written by the agent; it says what the agent last believed. Liveness is an **observation** the host makes; it says whether the process is alive and producing.

The host must expose both, never merged, and never let a consumer read only the claim:

- `claimed_state` — the slot value, agent-authored, with its step token.
- `observed_state` — host-derived: process alive, last output timestamp, child process inventory, current phase deadline.

A lane whose claim says `running` and whose last output is 3 hours old is a **typed condition** — `stale-claim` — that the supervisor raises without anyone noticing. This is the host-owned job supervisor already scoped as stint 0628; that stint is the load-bearing dependency of this whole PRM and everything else here is easier once it lands.

### 3. Structured returns from a pane agent

Today the head scrapes a 20-line screen tail and parses prose for a verdict. An agent pane must be able to return a **schema-validated JSON result**:

- The spawner declares a result schema at brief time.
- The agent writes its result once, through a typed result channel (not a slot, not scrollback).
- The host validates against the schema and rejects a malformed result *to the agent*, so it can correct before the supervisor ever sees it.
- `plexi pane result <id>` returns the validated object or a typed error.

The tester verdict is the obvious first schema, and its shape matters for a failure mode below: a verdict is not a string, it is `{verdict, criteria: [{id, source, met, evidence}]}` where `source` cites where in the task body the criterion came from.

### 4. Addressable resume

A pane must be addressable by durable name across host restart, and re-briefable by that name without the caller holding an id. The babysitter already labels every pane and then tracks bare ids anyway, because names are a display affordance rather than an identity. Names become identity.

### 5. Worktree isolation as a primitive

`plexi pane new --worktree <branch>` creates the worktree (through `wtp`, per repo policy) and reclaims it on close when nothing changed. Today this is several steps of prose inside `implement-stint` Phase 2, and a step of prose reaping in the head. One flag.

### 6. Resource leases and machine-pressure admission

The OOM was not caused by a wrong number in a config file. It was caused by the number living somewhere only an agent's discipline enforced. The host owns admission:

- `plexi lease acquire <name> --max <n>` / release, held for a process lifetime and reclaimed when it dies. Lane slots and the cargo-build lock are both leases.
- A machine-pressure signal the supervisor gates on. **It must measure swap-file growth and concurrent cargo/rustc count, not free-memory percentage** — the live incident proved free% sat flat at 32–33% all night while macOS grew the swap file from 13G to 31G underneath it. Free percentage is a deceptive lagging metric and no admission decision may read it.

---

## Where the run's state lives

`RUN_STATE.md` exists because a head's context dies. Once the run record is on disk, that motivation is gone and the file changes role.

**The durable run record is an append-only event log**, one run per directory, under the channel profile: `~/.plexi-<channel>/runs/<run-id>/`.

- `run.toml` — the immutable config snapshot taken at run start (engine policy, caps, authorizations, reservations). Snapshotting matters: a run must not silently change behavior because someone edited `RUN_CONFIG.toml` mid-flight.
- `events.jsonl` — append-only, the **single source of truth**. Every lane transition, spawn, verdict, merge, override, park, and throttle action is one line. Newline-delimited JSON, per commandment 1: portable, greppable, and readable in a hundred years.
- `lanes/<lane-id>.json` — a materialized view, a fold over the event log. Derived, deletable, always regenerable.

Everything else becomes a rendering:

- **`RUN_STATE.md` becomes a generated human-readable view** of current state, never authoritative and never hand-written. It stops being a baton because there is nothing to hand off.
- **`LOG.md` becomes a rendering of the event log**, so telemetry stops depending on a head remembering to append.
- **`HUMAN_CHECKS.md` stays hand-maintained.** It is a human's to-do list, its checkboxes are human input, and it should not be machine-owned.

**Crash survival:** the event log is fsync-appended before each side effect is taken, so a crash loses at most one in-flight action and never loses the record of one that completed. Panes outlive the supervisor — they are host-owned processes — so a supervisor crash orphans nothing.

**Resume means reconcile, not replay-and-trust.** On restart the supervisor folds the event log to a believed state, then **verifies that belief against the world** before continuing: pane liveness by name, PR state via `gh`, task state via `stint show`, installed-build head sha. The current skill says the opposite — "trust the baton, it exists so you start clean" — and that instruction is correct only because the baton is all a fresh head has. With a real record it becomes wrong: the baton says what was intended, the world says what happened, and where they disagree the world wins.

---

## Failure modes the design must close

Each of these was observed live. A design that does not close it by construction is not done.

**A head wedged 3h10m while its slot read `state=running`.** Closed by primitive 2: the claim and the observation are separate fields and a supervisor raises `stale-claim` from the observation. Additionally, no lane may be in a phase without a deadline — a phase with no bounded duration is a design error, not a long task.

**A status slot that is a claim, not liveness.** Same primitive, stated as an invariant: *no consumer may make a scheduling decision from `claimed_state` alone.* This is enforceable in code — the API that returns a claim returns it alongside its observation, so reading one without the other is not expressible.

**~17 lanes each running `cargo build --release` into OOM.** Closed by primitive 6: caps are host-held leases, not remembered numbers, and admission gates on swap growth and build-process count. `prewarm_release_build` becomes safe to re-enable the day the build lease exists, because the thing that made it dangerous was that nothing serialized it.

**A merge runner correctly refusing conflicts, with the conflicts then needing a head.** A conflict is a **typed lane outcome**, not an escalation performed by prose. It transitions the lane to `needs-judgment` — a first-class queue with a reason, the artifacts, and the blocked lane attached. Judgment work being queueable rather than shouted is what lets a supervisor keep running while a conflict waits, and it is the same queue that receives disputed FAILs and parked questions. The `merge_runner_escalates_on` list becomes the set of outcomes that route here.

**Stint id collisions from untracked task files.** Id allocation is `ls | grep | sort | tail -1` — a read-modify-write with no lock, run concurrently by many lanes. It must become an atomic reservation owned by the `stint` CLI (a reserve-then-write verb under a lockfile), the same way `stint claim` is already lock-protected. **Note the repo tension:** `.stint/` is gitignored by design (`AGENTS.md`, "Tasks and Issues"), so "commit the task files" is not available here as a mitigation — which makes atomic allocation the only fix, not a nicety.

**A tester producing a false FAIL from a badly-worded mechanical criterion.** Closed structurally by primitive 3's verdict schema. Every criterion carries a `source` citing where in the task's Done-When it came from. A criterion with no source is rejected at brief-construction time, by code, before a tester ever sees it — which makes "never write a grep-returns-nothing proxy for a semantic requirement" an invariant instead of a paragraph of prose the head must remember. Overruling a FAIL remains judgment, and the override is recorded in the event log with its reason.

---

## Migration

The babysitter is load-bearing and runs nightly. Every step below is independently valuable, ships behind the prose skill continuing to work, and deletes a specific workaround when it lands. No step requires the next one.

1. **Instrument first, change nothing.** The prose head appends the run's events to `events.jsonl` alongside everything it already does. Zero behavior change. This buys real data on phase durations and failure frequency, and it validates the event schema against a live run before any code depends on it.
2. **Pane lifecycle events + `pane wait`** (primitive 1). The skill's polling loops become single blocking waits immediately. Deletes the hand-built `until` loops and demotes `check_interval_seconds` to what it already claims to be.
3. **Observed liveness / job supervisor** (primitive 2, stint 0628). Deletes the wedge class and the `RUN_CONFIG` note that head-engine choice is a mechanics exception — a Codex head becomes viable the day liveness is a host invariant rather than an agent capability.
4. **Structured returns** (primitive 3). Deletes verdict scraping, the capture-forms cost table, and the sub-agent-for-long-reports escalation.
5. **Leases and admission** (primitive 6). Deletes `[limits]` as an honor system.
6. **Run record + `plexi run` CLI** — create, show, resume — with the prose head still driving. State moves off context onto disk. `RUN_STATE.md` becomes generated.
7. **The control loop moves into code.** A supervisor executes the deterministic column, and calls a model agent for each judgment step with a typed question and a typed answer. This is the last step, not the first, and by the time it arrives it is small.

**Gate between steps 6 and 7:** run both paths against the same queue for one night and require the code path to reproduce the prose path's outcomes. The prose orchestration is retired only after that, and `SKILL.md` is then cut down to the judgment brief library — not deleted, because the judgment column does not go away.

---

## Not in scope

- **A general-purpose workflow DSL for users.** One consumer does not justify an engine. It becomes in scope when a second pipeline (release, marketplace review) wants the same supervisor and the babysitter's own shape has been stable for a full sprint.
- **Distributed or multi-machine fleets.** That is Plexi Server, a separate product on the other side of the local/server split. In scope once the single-machine supervisor is proven and the server identity model exists.
- **Automatic merge-conflict resolution.** Routed to judgment, permanently, until there is evidence a model resolves them without silently destroying state — and the app-state migration conflicts in this repo are precisely the case where a wrong resolution is invisible.
- **Removing the human from `HUMAN_CHECKS.md`.** In scope per item, only when a host primitive makes that specific check drivable (audio output capture, a fullscreen CLI verb, a working framebuffer gate).
- **Replacing the tester round with static analysis or CI.** CI green means nothing red, not reviewed. In scope when a review layer actually reads the diff.
- **Porting the reference harness's workflow layer verbatim.** Its agents have no host to attach to; Plexi's testers do. The three contracts worth adopting are pushed completion, structured returns, and isolation-as-a-flag — all specified above as host primitives.
- **Automating tier and engine selection.** Judgment, cheap to get right by hand, expensive to get wrong.

## References

- `.agents/skills/babysitter/SKILL.md` — the contract this PRM decomposes
- `.agents/skills/babysitter/RUN_CONFIG.toml` — run-invariant settings that become `run.toml` and host leases
- `.agents/skills/implement-stint/SKILL.md` — Worker Mode; the pane-slot write contract that becomes a structured return
- `AGENTS.md` — Inter-Pane Communication (the event bus that carries pane lifecycle events); Tasks and Issues (`.stint/` gitignore)
- `NORTH_STAR.md` — commandments 1, 4, and 10; the local-first and CLI-completeness constraints this design is bound by

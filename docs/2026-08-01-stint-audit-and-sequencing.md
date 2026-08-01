# Stint audit and sequencing to zero

Date: 2026-08-01. Status: proposal, awaiting owner ruling.
Companion to [`2026-08-01-v1-cut.md`](2026-08-01-v1-cut.md), which remains the authoritative V1 node ordering. This document does not replace it. It amends it for the Assistant/mesh pull-forward, records the close list, and sequences everything else.

Method: five parallel read-only auditors over all 172 open tasks (497 total, 324 done, 7 archived), partitioned by area with no overlap. Shipped-ness proven only with `git merge-base --is-ancestor <sha> origin/alpha` — never `git log --all`, which includes unmerged branches and produced one wrong audit before the rule was imposed.

---

## 1. The headline

**The stint graph is not the bottleneck. The merge queue is.**

At audit time there were **15 open PRs, the oldest from 2026-05-21**. Several carried passing checks and were merely `CONFLICTING` from age — they rotted while waiting, they were not wrong. Two were `CLEAN` and had simply never been merged.

Across 172 open tasks the audit found **two stale closes, one duplicate, and no contradictions**. The graph is dense and honest. The expectation that it was full of superseded junk was not borne out.

The corollary: a large share of the 21 `in-progress` tasks are not stalled work. They are finished code queued behind a gate.

---

## 2. Scope ruling in force

The owner ruled on 2026-08-01 that **the Assistant app and the agent-swarm/mesh infrastructure are V1 launch scope.**

This supersedes, on the Assistant and orchestration points only:
- `2026-08-01-v1-cut.md`: *"Do not pull Assistant, orchestration, browser, marketplace, native WASM distribution, or media editing into this sprint."* Browser, marketplace, WASM distribution, and media editing **stay out**.
- `assistant-agent-mesh.md`: *"Nothing in this document may be pulled forward into the v1 acceptance set."*

Both documents need editing to match. Until they are, agents will keep reading a stale constraint — one auditor correctly refused a peer-relayed override for exactly this reason.

**The Assistant is a host app, not a PGAP app.** `assistant-host-app.md` is canonical; `NORTH_STAR.md` is inaccurate on this point and should be corrected. Code lives in `src/assistant/` and `src/app/assistant_host_tools.rs`. The only `apps/`-side remnant is `apps/dev/assistant-pgap/`, an empty shell holding one stray `hello-world/SKILL.md`, absent from `packs/core.toml`. It should be deleted.

---

## 3. Close list — apply in bulk

| ID | Action | Proof |
|---|---|---|
| 0659 | CLOSE-STALE | `superseded_orphans` / `SupersededOrphanPolicy` / `reclaim_python_state_orphans` all deleted by `171a6903` (PR #2545), confirmed ancestor of alpha. Both variants gone; the question is moot. |
| 0663 | CLOSE-STALE — **needs second confirmation** | An auditor ran `headless_frame_fails_fast_when_the_guest_dies_at_import` on clean alpha at `d20cf75f` and reports it passes. If confirmed, the `--skip` line and its comment in `rust-host.yml` are obsolete. Do not act on one observation. |
| 0580 | DUPLICATE-OF 0662 | Same test, same race, filed nine days later. |
| 0439 | CLOSE-SUPERSEDED-BY 0554, 0628 | Asks for a design spec that already shipped as `assistant-authority-model.md` (stint 0554, done) and `agent-run-orchestration.md` (stint 0628). Keep 0628 as the live tracker. |

**Needs definition before dispatch, not closure:** 0007, 0249, 0256, 0263, 0266, 0267, 0268, 0395, 0598. Each is either a one-line body with no acceptance criteria or a task whose own notes flag it stale. 0395 is a special case — see §6.

**Unverified, cannot be settled from this repo:** 0329 (depends on external `nooise` release state).

---

## 4. Corrections to the graph

Status and metadata fixes, none of which change scope:

- **0641, 0652, 0638, 0651 are correctly `in-progress`.** An earlier audit claimed they had shipped and were stale bookkeeping. They had not — the cited commits live on unmerged branches. Do not mark them done.
- **0646** carries a `blocked_by` edge on 0644/0652 that never gated it; it merged as #2534 regardless. Drop the edge.
- **0644, 0645** are genuinely parked, stacked behind PR #2536.
- **0486** → REPRIORITIZE p1. It sits between two p1 tasks and gates 0487; leaving it p2 strands it behind unrelated work.
- **0520** → REPRIORITIZE p2. Its own `v2` tag and sprint placement contradict its p1.
- **0707** → move `backlog` to `todo`. Well-specified, unblocked, and actively destructive today.
- **0382, 0441** → REPRIORITIZE up (p1, p2 respectively); both sit under now-V1 planes.
- **Missing dependencies not declared in `blocked_by`:** 0599 after 0441; 0333 after 0558/0249; 0490 after 0482; 0593 after 0592; 0635 after 0633; 0602 after PR #2503 and 0596; 0673 after 0672 (needs a night of its data).

**Sprints:** the graph tracks only 133 of 497 tasks (27%); 454 are unsprinted including 37 open p0/p1. s4/s9 and s5/s8 are genuine parallel lanes, not botched re-plans. s10 is fiction — 24 tasks, half unfinished, dated to a single day. Recommendation: keep s2 live, archive the four 100%-done sprints, **dissolve s10**, and stop sprinting new work. The field is not earning its keep; `create-stint` has already been amended so no agent assigns one.

---

## 5. Do first — the force multipliers

These make every other task cheaper. They are not product.

1. **0710 — DONE**, merged as `d371dc6a`. CI went from 10m12s to 8m28s on a cold cache with the `build` job deleted; a docs-only commit now costs zero.
2. **0637 + 0657 together.** `pr-install` does duplicate compute and substitutes a stale SDK. Every babysitter and tester cycle runs through it. Fix jointly — 0637's reuse-skip masks 0657's bug.
3. **0547.** Tiered validation policy. Removes the mandatory human-tester round for pure-logic batches. This is what lets a swarm run unattended.
4. **0670.** Stint IDs are allocated with `ls | sort | tail -1`, no locking. Two concurrent agents get the same ID. This session ran eight agents at once and got lucky.
5. **0690.** Host-arbitrated build lock. Its absence is documented in the babysitter LEARNINGS as a repeated, real violation.

---

## 6. V1 set

Ordered. Merge-queue items first, because they are finished work.

### 6a. Merge queue — finished code, needs only landing

`#2522` merged (0607). `#2556` merged (0710). Remaining: **#2557** (0654), **#2532** (0638/0641), **#2536** (0651/0652 → unblocks 0644/0645), **#2555** (0591/0701/0702/0703), **#2503** (0590/0441), **#2533** (0653), **#2499** (0382, parked on 0601), **#2489** (0549/0550/0551, V3).

Older PRs — **#1604, #2316, #2318, #2323, #2353** — predate this audit's window and were not assessed. Triage or close them; a PR from May is not a plan.

### 6b. V1 product work, by dependency

**Security and correctness**
- **0577** (p0) — no `sandbox_init`, `seatbelt`, or `sandbox-exec` anywhere in the tree. Untrusted MCP subprocesses run with ambient authority, violating commandment 10. Also blocks #2499. **Conflicts with the v1-cut doc, which lists sandboxing as Post-v1. Owner must rule — see §8.**
- **0707** — `plexi ai setup` destroys `[ai.local]` and budget caps via a hand-rolled line scanner at `src/cli/ai.rs:492`. Corrupts the Meridian config on alpha today.
- **0636** — `plexi notify` takes `source_pane_id` as a caller-supplied parameter rather than deriving it from the socket peer. Sender identity is spoofable, at exactly the moment the mesh multiplies cross-agent messages.
- **0693** — `plexi notify` hardcodes `priority: 50` at `src/cli/notify.rs:98`; the audible cue never fires from the CLI.
- **0680 → 0682 → 0681 → 0683** — Python app state silently corrupts and clobbers today. Do as one cluster.
- **0601** — unbounded stdin buffer growth; a slow OOM under long-running agent MCP clients.

**Host stability — a swarm needs a host that survives hours**
- **0694 + 0705** — same heartbeat freeze/thaw signature. **One investigation, not two parallel dispatches.**
- **0548** — UI-thread-blocking operations (blocks 0553).

**Mesh substrate**
- **0664 → 0665, 0666** — pane lifecycle events, claimed-vs-observed liveness, schema-validated results.
- **0625** — pane message mailbox.
- **0668** — worktree isolation flag.
- **0709 → 0628 → 0667** — liveness primitive, orchestration jobs, resource leases/admission.
- **0653** (#2533) — the MCP tool bridge the Assistant calls tools through.

**Release gates** — 0701, 0591, 0590 → 0596 → 0602, 0704, 0706, 0662, 0633 → 0482 → 0490, 0358.

**Agent-facing infra** — 0637+0657, 0547, 0670, 0672, 0690.

**Product surfaces** — 0696 (theme delivery, in-progress), 0699 **before** 0700 and before the 0661/0684 refactors, 0581, 0697, 0647 (after 0644), 0597.

### 6c. V2 / V3

**V2** — browser cluster (0480→0483→0486→0487→0489, 0484, 0485, 0488), terminal ergonomics (0258, 0259 — both dispatchable now, see §7), consent-UI test infra (0592→0593), CI hardening (0269, 0270, sequenced after 0710), secrets follow-ons, process hygiene (0560+0689), 0673, 0671, 0651/0652.

**V3** — all monetization (0322, 0341, 0344, 0286, 0323, 0354, 0356, 0352, 0353), the video-editor suite (0521, 0524, 0525, 0526), 0622, 0609, 0623, 0248, 0251, 0265, 0406, 0360, 0287, 0550, 0551.

**Drop list for V1: 9 marketplace/payments tasks, 4 video-editor tasks, and the external-project demo.** Third-party monetization was already deferred post-v1 by the team's own 2026-07-24 decision.

---

## 7. Traps this audit found in the graph itself

- **0258 and 0259 declare `blocked_by: []` but their prose cites a blocker that already shipped** (the `PlexiInput` router, stint 0387, now `src/app/input_router.rs`). An agent reading the prose skips them forever. They are dispatchable today.
- **0706 is not superseded by #2555** — that PR's own body explicitly defers it. Marking it closed would silently drop a v1 gate.
- **Three distinct MCP protocol extensions are in play** — tools (0382/0578), Apps (mesh #11), Tasks (mesh #12). Do not let anyone merge them into one stint.
- **0483 (native WebView runtime) and the mesh's MCP-app pane may be the same primitive built twice.** Rule before either is stinted.

---

## 8. Decisions only the owner can make

1. **Is sandboxing (0577) V1?** The v1-cut doc says Post-v1, citing NORTH_STAR's definition of v1 security as consent + audit + review. The audit calls it a p0 hole that also blocks #2499. Both cannot be true.
2. **Which surface is the flagship demo?** 0598 is a pure decision-blocker and now sharper with the Assistant in V1.
3. **Where do notification affordances live** — sidebar or dedicated chrome? 0699 and 0700 touch the same real estate, filed independently. Ruling: 0699 first, 0700 inherits.
4. **0395** — "third-party agents as first-class panes" is the mesh vision itself, but is scoped explore-only. Calling it V1 while leaving it in explore state is a contradiction.
5. **Webview ownership** — mesh MCP-app pane vs. `browser-surface.md`. The mesh doc names this as a ruling to take before either is stinted.
6. **0356 cannot be worked by any agent.** It is an ops checklist needing production credentials. It is a human gate wearing a task's clothes.

---

## 9. The agent mesh — 14 stints that do not exist

Nothing in `.stint/` decomposes the mesh, and it is now V1. This is the largest unscoped V1 workstream.

**Hard prerequisites — file these before any other mesh stint:**

- **P0. Context-root uniqueness ruling + fix** (M). `context-root-uniqueness-and-rollup.md` exists but **no stint references it**. Per-context heads collide if two contexts share a root. Blocks everything addressed by context root.
- **P0b. Sandboxed webview pane primitive** (L). No webview crate in the dependency set. Section 7 of the mesh doc is fully inert without it, and the doc says it "does not partially land."

**Then, in dependency order:**

| # | Stint | Size | Depends on |
|---|---|---|---|
| 1 | Head registry + addressing (keyed by root path, not `context_id`; `plexi assistant` namespace) | M | P0 |
| 2 | Drain hardening for assistant records (stable ids, lossless writes, version tag) | M | — (can start now) |
| 3 | Working memory + eviction-to-pointer | S | 2 |
| 4 | `ask_question` tool + capability-card aggregation, routing, cycle detection | M | 1, 2 |
| 5 | Behavioral test gate vs local Meridian backend (fixes reasoning-effort silently dropped on `LocalOpenAiBackend`) | S | 1, 4 |
| 6 | Routines address a head | S | 1 |
| 7 | Unprompted messages + per-head budget | S | 1, 2, 6 |
| 8 | Escalation as a typed record | M | 2, 4 — **share schema with `decision-trust-plane.md`** |
| 9 | Conversation-surface unification (`Turn` gains stable id + author; head owns the store) | L | 1, 4 — split into 9a/9b if a babysitter queue needs smaller units |
| 10 | Command-palette head picker | S | 1, 9 |
| 11 | MCP Apps host-side plane (`ui://`, postMessage bridge, visibility gating) | L | P0b, 2, 0696, 0699 |
| 12 | Long-running work via MCP Tasks extension | M | 6, 0628 |

**Already covered — do not re-file:** 0628 (feeds #12), 0554 + `assistant-authority-model.md` (the authority plane the mesh explicitly does not change), 0696 and 0699 (feed #11), 0382/0578 (distinct MCP layers), 0558 (feeds #4's authority guarantee).

**Explicitly out of scope per the doc:** A2A/ACP wire-protocol compatibility, per-agent private logs, embedded-vector memory.

---

## 10. Recommended order

1. **Land the merge queue.** It is finished work and it is three days cold.
2. **Force multipliers** — 0637+0657, 0547, 0670, 0690.
3. **Owner rulings in §8.** Several block filing, not building.
4. **File the 14 mesh stints.** P0 and P0b first.
5. **V1 security and correctness** — 0577 (pending ruling), 0707, 0636, 0693, the 0680-0683 cluster.
6. **Host stability** — 0694/0705 as one, 0548.
7. **Mesh substrate** — 0664 chain, 0625, 0668, 0709→0628→0667.
8. **Release gates**, then acceptance.

Everything V2/V3 waits. The drop list is 30+ tasks and it is the most valuable output of this audit.

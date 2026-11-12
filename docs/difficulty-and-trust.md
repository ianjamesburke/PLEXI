# Difficulty and Trust for Work Allocation

Status: proposed. Post-1.0 idea with no schedule; not active, because no stint owns it yet.
Stint: none yet. File one only after the owner answers the questions in section 7.
Supersedes nothing. Borrows two rules from [`decision-trust-plane.md`](decision-trust-plane.md) (superseded) and drops the rest; see section 5.
Last updated: 2026-11-12.

## 1. The problem

Whoever assigns a stint picks a model and a review depth by rule of thumb, and nothing records whether the pick was right, so easy work gets an expensive model and hard work can get too little review.

The user is the person allocating work: today the owner or the lead session, later the host doing it for them. The standing rule they apply is: the strongest model for size `l` or the hardest work, a mid model for `m` and `s` implementation, a small model for filing and mechanical edits; a critic plus an independent installed tester for size `l` or `risk-high`. It is a reasonable guess. This proposal replaces the guess with a record.

## 2. Difficulty

**Measured after, estimated before.** `size` stays the estimate, set by judgement when the stint is filed (`.agents/create-stint.md`). Difficulty is what the finished stint actually cost. Calibration is how often the estimate matched the measurement. The two never overwrite each other.

Measures to record on every finished stint:

- **Agent-active time.** The sum of the time an agent was working, read from the harness session, not `started_at` to `completed_at`. Time with an open `needs-you` row or a PR waiting on the owner is excluded.
- **Critic rounds.** How many critic verdicts it took to reach PASS.
- **Send-backs.** An independent tester's FAIL, a PR returned by the owner, or a PR closed and restarted (the failed PR reset in root `CLAUDE.md`).
- **Breadth.** Files and `area` values touched by the merged diff.
- **Risk class.** The existing `risk-*` tag.
- **Tokens and model.** Input and output tokens, and the agent definition, model and harness that did the work.
- **Installed test.** Whether an installed-build test was needed, and whether it found something the local gates missed.
- **Incidents.** A revert or a follow-up bug traced to this stint after merge.

Do not measure lines of code (deletion is often the right fix), commit count (checkpoint commits and rebases inflate it), or PR open-to-merge time (it measures the reviewer's day).

A single difficulty number is premature. Record the measures first; derive bands once there is enough data to see where stints cluster.

**Smallest change to the stint record:** a typed `outcome` table in stint frontmatter, written by the implementing skill when it marks the stint done, and checked by `stint check`. A missing field is an error, not a silent blank, so gaps in the data are visible.

## 3. Trust

**Whose trust.** Trust is recorded per agent definition, model and harness together. `src/agent/AGENTS.md` makes the definition the agent and the harness a property of it, but the same definition does very different work on a small model and a frontier one, so the record keeps all three and can be summed up to any one of them.

**What the record holds.** For each difficulty band: first-pass success (merged with no send-back), send-backs, critic rounds, and incidents. Trust is computed from these outcomes, never stored as an editable number. That rule comes from `decision-trust-plane.md` and survives: a number that can be edited was not earned.

**Earned and lost.** Trust in a band rises with clean outcomes in that band and falls faster than it rises: one incident costs more than one clean merge earns. Old outcomes count for less over time. A new model version starts low, because it is a different worker.

**What it gates.** Only how much review a stint gets above the floor its risk tag already sets:

- Whether a critic is required on `risk-normal` work in a given band.
- Whether a stint the rule of thumb would give a strong model can go to a cheaper one.
- Whether the owner's look before merge can be replaced by pre-authorization for that band.

The risk tag is the floor and trust never lowers it: `risk-high` always gets its installed tester (`.agents/create-stint.md`).

**What it never gates.** Authority. Grants, secrets, the agent sandbox profile and every approval stay enforced by the host whatever an agent's record says. Only a real click in the host window approves a `needs-you` row, and no socket, terminal or agent pane can (`src/cli/AGENTS.md`, `plexi needs-you`). The permission monitor never reads a trust record. The earlier tier sketch tied higher tiers to auto-merge and production deploys; that is authority, and it is out.

## 4. Allocation

Given a stint's size and risk tag, pick the cheapest agent whose trust in that band clears the bar; if none does, take the next stronger one. A small, fixed share of work goes to a cheaper candidate under full review, because without that a cheap model can never earn a record.

Calibration comes from fixed task sets, not only from live work, since live stints never repeat:

- **Plugin gauntlet** (stint 1262): ten plugin cards with the same prompts and rule-based scoring, plus an independent judge. Its rerun report (`.stint/runs/gauntlet-1262-rerun/REPORT.md`) already measured pass rates, tokens and wall time per card.
- **Plugin benchmark** (stint 1263): the same cards across models and harnesses, and its rider to replay merged stints from their parent commit and score them against the tests that merged. That replay set is the closest thing to host-maintenance work with a known answer.

Stays a human call: the risk tag, priority, what counts as an incident, any change to a bar, and an override on any single stint.

## 5. What exists and what does not

Exists:

- Stint frontmatter: `size`, `priority`, the `risk-*` tag, `started_at`, `completed_at`, and `actual` (elapsed claim-to-done time, not working time). The schema also has `estimate`; it was used briefly and abandoned, and hour estimates are now banned for Plexi stints (`.agents/create-stint.md`).
- `plexi ledger`: tokens, cost and wall time for AI calls the host brokers, per client. It does not see Claude Code or Codex sessions running in panes.
- `RunRecord` in `src/agent/runs.rs`: input and output tokens per agent run the host spawns.
- `needs-you` rows with timestamps, which can mark owner-wait intervals.
- The gauntlet report and the benchmark stint above, as a calibration seed.

Does not exist: the model, harness or tokens per stint; a count of critic rounds or send-backs; owner-wait accounting; any link from an incident back to the stint that caused it; any trust record; any allocator.

From `decision-trust-plane.md`, keep two rules: trust never widens authority, and trust is derived from records, never stored. Drop the rest. It scored judgment calls on the retired head-agent mesh, it relied on the filing agent to estimate its own blast radius, and it designed the scoring before a single outcome had been recorded.

## 6. Risks

- **Gaming the measure.** An agent splits work to look fast, or closes a stint before the hard part. Count follow-up stints filed from the same work against the original.
- **Trust hiding a regression.** A long clean record keeps a model on work it now fails, for example after a silent provider update. Old outcomes fade, a version change resets, and incidents weigh more than successes.
- **Measuring the owner by accident.** Elapsed time mostly measures when the owner was at the desk. Use agent-active time only, never claim-to-done.
- **Premature precision.** A score to three decimals on a handful of stints is noise. Show counts and plain bands until there is volume.
- **Scope creep into authority.** Any proposal that lets trust skip a click or widen a grant is rejected on sight.

## 7. First experiment and open questions

**Experiment, after 1.0:** for a fixed window of normal work, every implementing skill writes the `outcome` table. Nothing is gated and allocation stays the rule of thumb. At the end, one report: does `size` predict measured difficulty, and which model and harness pairs had a clean first pass in which bands. Run the stint 1263 matrix once in the same window for a controlled comparison. Decide on gating only after reading that report.

Questions for the owner:

1. Where does the outcome record live? Recommended: typed fields in stint frontmatter, so the stint stays the one record of the work.
2. Is trust per agent definition, per model and harness, or both? Recommended: record all three together and sum up as needed.
3. May trust ever remove the owner's look before merge? Recommended: only for `risk-low` and `risk-normal` in bands with a long clean record, and never for `risk-high`.
4. Does the old standing ruling that trust is a continuous number, never levels, still hold? Recommended: no; show plain bands until there is enough data for a number to mean something.
5. What counts as an incident? Recommended: a revert, or a bug the owner tags as caused by a stint; nothing inferred automatically.
6. Should cheap models get a share of work just to earn a record? Recommended: yes, a small fixed share, always under full review.

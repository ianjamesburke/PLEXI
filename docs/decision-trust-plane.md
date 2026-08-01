# Decision Trust Plane

Status: active.
Stint: none yet — filed alongside the mesh PRM's first sprint.
Sibling: [`assistant-agent-mesh.md`](assistant-agent-mesh.md).
Authority plane: [`assistant-authority-model.md`](assistant-authority-model.md).
Last updated: 2026-08-01.

This document owns one question: **when a judgment call has more than one defensible answer, who gets to answer it?**

It exists as its own document because the mesh PRM deliberately excludes the authority plane, and this is neither the mesh nor the authority model. The mesh describes how heads coexist, remember, and talk. The authority model describes what any of them is permitted to *do*. This describes whose judgment stands when the question is not "may I" but "which of these three".

Sequencing is the mesh PRM's: post-v1, Phase 3. This depends on the mesh's drain (§3 there) for storage, which in turn carries the hazards that document's appendix enumerates. No part of this can be built on a drain that silently drops records, because a lost decision record is a trust score computed from a lie.

## The line this document must not cross

**Rising trust widens decision delegation. It never widens authority.**

This is stated first because it is the only way the rest of the document is safe. The two planes are independent and intersect nowhere:

- **Authority** is what an agent may reach — which tools, which files, which resources. It is decided per call by the host reference monitor, against a grant that names an exact actor, resource, and argument set. [`assistant-authority-model.md`](assistant-authority-model.md) owns it. **Nothing in this document modifies it.**
- **Trust** is whose judgment resolves an open question about how to proceed. It is earned, per category, and it moves.

A head with the highest trust in the system has exactly the tool access a brand-new head has. Every one of its calls is checked identically. **"Auto-approve" here means "this agent's judgment stands", never "this agent skips permission checks."** The reference monitor does not read trust scores and must not be given the ability to.

Two corollaries, both load-bearing:

- **A decision record can never be a request for a capability.** If resolving a decision would require an action the reference monitor would deny, the decision is not the mechanism — the grant is, through the permission path, from the user.
- **Conflating the two reopens exactly the confused-deputy path the mesh closes.** The mesh rules that `ask_question` returns information and never authority, so that asking a more capable head is not a way to borrow its capabilities. The same rule applies here: a high-trust hop resolving a decision confers no capability on the agent that filed it.

## 1. The decision record

When an agent reaches a judgment call with a **large blast radius and multiple viable implementations**, it does not decide. It files a decision.

The record is a schema, not a convention — an incomplete decision is not fileable. This is the same discipline the mesh applies to escalations, for the same reason: a rule that says "include your reasoning" is followed until the night it matters.

Required fields:

- **The question**, in plain language, standing on its own without the filer's context.
- **The options**, each with what choosing it actually means — not labels, not internal identifiers.
- **The filer's prediction**: which option it believes is right, and a probability.
- **Blast radius**, a float.
- **Category**, naming the kind of judgment this is. Categories are the axis trust is scored on, so this field is what makes trust mean anything.

**The prediction is the point of the whole record.** Without a prediction logged *before* the answer is known, there is nothing to score, and trust cannot be earned — it could only be asserted. The prototype already works this way: Ian's decision log records a float `confidence` and a `predicted_outcome` at filing time, then records what was actually chosen afterward, precisely so a running success rate exists at all. That prototype's fixed integer axes do not carry forward; its predict-then-record loop is the part that does.

**The prediction doubles as the default.** If a hop resolves without substituting its own answer, the filer's prediction stands. This matches how the babysitter ladder already works in practice — a head ruling on a worker's ask defaults to the worker's recommendation — and it means filing a decision is never a way to stall. A filed decision is a real end state, satisfying the mesh's rule that no agent ends a turn in limbo.

## 2. Resolution rises

A decision travels **worker → head → human**. At each hop, one question: is this hop's trust *in this category* at or above the threshold that this blast radius requires?

- **Yes** — the hop may resolve it. It records that it resolved, and its trust at that moment, so the fold in §3 can later score the resolution itself.
- **No** — the record rises unchanged, carrying every hop's annotations. Nothing is stripped on the way up; a human seeing it last sees the whole path.

Four rules govern the climb:

1. **Rising is the default.** Resolving is the exception trust buys. An agent that cannot tell whether it is above threshold is below it.
2. **No hop may lower a blast radius set by another.** Otherwise the ladder launders its own escalations, and the first thing a miscalibrated agent learns is to mark everything small.
3. **Ambiguity rises.** Same fail-visible principle the mesh applies to ambiguous question routing: when it is unclear who should answer, a person answers.
4. **Some categories never delegate, at any trust.** Money, irreversible actions, spec reversals, and taste calls go to the human regardless of how well calibrated an agent has become. This is a declared floor that trust cannot raise, and it is the one place in this design where a number is not permitted to win.

**This supersedes the prose decision ladder in the babysitter skill**, which is its prototype and which already has the shape right: worker first, head second defaulting to the worker's recommendation, human last for money, irreversible actions, spec reversals, and taste. What that ladder lacks is enforcement — it is a sentence in a skill file, and the mesh appendix's own argument applies without modification: when repeated instruction fails to hold a line, the fix belongs in the host, not in another rewording. The `file-gate` skill's gate stint is the current implementation of "a worker stops and asks"; the decision record is its typed successor.

## 3. Trust is a fold, never a maintained number

**Trust and blast radius are continuous floats from 0.0 to 1.0. Never categorical levels.** This is a standing ruling and it is binding on every surface here: a level is a decision made once by whoever named the buckets, and this system's entire purpose is to let calibration be measured rather than declared.

**Trust is derived, never stored.** It is a fold over resolved decision records in the drain — nothing writes a trust score, and there is no trust file. If a number can be edited, it was not earned. This is the same rule the mesh applies to capability cards: the aggregate is computed from the underlying records so it cannot drift from what it claims to describe.

What the fold weighs:

- **A prediction that matched the eventual outcome raises trust in that category.** One that missed lowers it.
- **Verified outcomes outweigh chosen ones.** A human picking the predicted option is evidence; the choice later proving correct is stronger evidence. Both count, not equally.
- **A decision with no recorded outcome contributes nothing** — not zero, nothing. Unresolved records must not drag a score toward the middle, or an agent could lower its own visible trust by filing.
- **Evidence decays with age.** A calibration that never decays cannot recover from a genuine improvement, and cannot lose credit earned long ago on a codebase that no longer exists.

**New agents start low. Always.** Cold start is not a special case to be optimized away; it is the mechanism working.

## 4. Thresholds are explicit configuration

The mapping from blast radius to required trust is declared config with **no defaults**. A missing threshold is an error naming the category and the file, never an implicit permissive value.

This follows the repo's standing configuration rule, and it matters more here than almost anywhere else: this mapping is the single number that decides whether a machine or a person answers a consequential question. A silent default would be a policy nobody chose.

## 5. Where this plane hooks in

- **The implementation workflow.** Workers file decisions instead of making judgment calls, and heads resolve or forward them. This is the concrete change to how sprints run, and it supersedes the prose ladder named in §2.
- **The drain.** Decision records and their outcomes are drain records, inheriting the mesh's requirements wholesale: stable ids, writes that fail loudly rather than silently, typed recall rather than reading a file off disk. Trust folds over exactly those records.
- **Escalation.** The mesh's escalation schema is **the human-facing end of this same record**. An escalation is a decision that reached the human hop — not a parallel mechanism with its own fields. One record type, two ends, so the thing a person reads is the thing the agents were passing around.
- **Receipts.** A decision rising past a head shows up in that head's conversation as a receipt, like any other unattended traffic, so the climb is visible where the work is rather than only in a log.

## 6. The agent creation surface

The destination for `plexi agent` is creating and configuring agents — the one thing the noun should mean, replacing the unrelated referents the mesh appendix inventories. Eventually it is an app rather than a command: memory on or off, tool grants chosen from the centrally enumerated MCP and tool list with pre-approved credentials linked at grant time, an initial permission scope, and an initial trust that is **always low**. Grants come from the enumerated list rather than a typed-in one for the same reason the mesh enumerates capability cards — a hand-written grant list describes an agent nobody verified exists. And creation never sets trust: a creation surface able to set it would be an edit path to a derived number, which is the one thing §3 forbids.

## Non-goals

- **This is not an authority mechanism.** Restated because it is the failure that would matter: trust never widens what an agent may reach, and the reference monitor never consults it.
- **No trust scores for third-party apps.** This plane scores the judgment of agents. Apps are governed by capability grants, and nothing here softens that.
- **No shared or cross-machine reputation.** Trust is local and personal to one Plexi instance, like every other record in it. An agent's calibration is evidence about how it performed here, and it does not travel.
- **No auto-resolution of the reserved categories**, however high a score climbs. §2's fourth rule is a floor, not a default.
- **No trust in the model's self-report.** A score is computed from recorded outcomes only. An agent stating its own confidence is a field in a record, never an input to its own trust.

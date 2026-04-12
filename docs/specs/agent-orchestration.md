# Agent Orchestration and Trust System

**Status:** Draft  
**Last updated:** 2026-04-11

---

## 1. Overview

Plexi's agent system is directory-scoped. Every terminal pane has a built-in terminal agent. Specialized agent networks (e.g., Parallax for video production) can be installed per-project. Agents coordinate through an orchestrator — never peer-to-peer — and trust grows organically from observed outcomes.

The core loop: user talks to the terminal agent, the terminal agent delegates to the appropriate orchestrator, the orchestrator decomposes the task and delegates to specialized sub-agents, results surface back to the user through the terminal agent.

Trust and risk are continuous floats (0.0-1.0), not categorical levels. The system self-tunes thresholds based on prediction accuracy, and builds an organic regression suite from real production work.

---

## 2. Agent Directory Structure

```
.plexi/agents/
  terminal/                      <- built-in, one per directory
    system.md                    <- terminal agent system prompt
    memory/                      <- accumulated learnings
    conversations/               <- conversation history
    config.yaml                  <- model, trust scores, thresholds

  parallax/                      <- installed agent network (example)
    config.yaml                  <- model assignments, trust scores, thresholds
    orchestrator/
      system.md                  <- orchestrator system prompt
      memory/                    <- workflow patterns, user preferences
      references/                <- reference docs the orchestrator can cite
      predictions.jsonl          <- prediction feedback log (Section 6)
    script-writer/
      system.md
      memory/
      versions/                  <- versioned snapshots (Section 7)
      test-cases/                <- auto-captured regression cases (Section 8)
    evaluator/
      system.md
      memory/
      criteria.yaml              <- quality scoring rubric
    improvement-officer/
      system.md
      memory/
```

**Rules:**
- The `terminal/` agent always exists. Plexi creates it when a directory is first opened.
- Agent networks (like `parallax/`) are installed explicitly — `plexi install parallax` or manual placement.
- Each agent owns its own `memory/` and `system.md`. No shared mutable state between agents.
- `config.yaml` at the network root holds cross-agent settings (model assignments, trust scores). Per-agent config overrides are optional.

---

## 3. Agent Communication

Agents communicate through the orchestrator, never peer-to-peer. Every message between agents passes through the orchestrator, which maintains the full workflow state.

### Delegation Flow

Example: "Make a 30-second ad for this product"

1. User talks to terminal agent.
2. Terminal agent recognizes this as a Parallax task. Delegates to Parallax orchestrator (HoP).
3. HoP generates a plan. Surfaces the plan to the user for approval (via terminal agent).
4. User approves. HoP delegates to script-writer agent.
5. Script-writer produces a script. Returns to HoP.
6. HoP delegates to storyboard-planner. Returns scenes.
7. HoP delegates to asset-generator (or runs tools directly). Stills generated.
8. HoP runs assembly tools. Video produced.
9. HoP delegates to evaluator. Quality scored.
10. Results surface to user via terminal agent.

### Delegation Mechanics

Each delegation is an LLM call with:
- The sub-agent's `system.md` as the system prompt
- The sub-agent's `memory/` loaded as context
- The sub-agent's available tools (declared in `config.yaml`)
- Task-specific context from the orchestrator (input brief, upstream outputs, constraints)

The orchestrator tracks which step the workflow is on, what each sub-agent returned, and whether the user needs to approve before proceeding.

### Terminal Agent Handoff

The terminal agent does not understand Parallax internals. It recognizes installed agent networks by scanning `.plexi/agents/*/config.yaml` for `trigger_patterns` — regex or keyword matches on user input that indicate delegation.

```yaml
# .plexi/agents/parallax/config.yaml
trigger_patterns:
  - "make.*video"
  - "produce.*ad"
  - "parallax"
  - "storyboard"
```

When a trigger matches, the terminal agent passes the full user message to the orchestrator and acts as a relay for any user-facing messages the orchestrator returns.

---

## 4. Trust Score System

Every agent has a trust score: a float from 0.0 to 1.0.

### Initial Trust

New agents start at 0.5 (neutral). At 0.5, every operation requires user approval.

### Trust Adjustments

| Event | Adjustment | Rationale |
|---|---|---|
| Successful operation, user didn't object | +0.01 | Slow organic growth |
| User explicitly approved | +0.02 | Slightly faster — active signal |
| User denied an operation | -0.05 | Fast penalty — wrong prediction |
| User undid an agent's action | -0.10 | Significant — agent caused rework |
| Error or unexpected outcome | Frozen | No adjustment until reviewed |

Trust is **per-agent, per-operation-category**. An agent can be highly trusted for writing files but not for deleting them.

### Config Format

```yaml
# .plexi/agents/parallax/config.yaml
agents:
  script-writer:
    model: claude-sonnet-4-6
    escalation_threshold: 0.60
    trust:
      global: 0.72
      write_file: 0.85
      delete_file: 0.30
      run_script: 0.55

  evaluator:
    model: claude-haiku-4
    escalation_threshold: 0.50
    trust:
      global: 0.68
      read_file: 0.95
```

### Trust Resolution

When evaluating whether to auto-approve an operation:
1. Look up operation-specific trust (e.g., `trust.write_file`).
2. If not found, fall back to `trust.global`.
3. Compare against the operation's risk score (Section 10) and the agent's `escalation_threshold`.

An operation is auto-approved when: `trust_score > escalation_threshold AND trust_score > risk_score`.

---

## 5. Orchestrator Prediction Model

The orchestrator doesn't just execute — it predicts. Before surfacing an approval request to the user, the orchestrator predicts: "will the user approve this?"

### Prediction Format

```json
{
  "prediction_id": "pred_001",
  "agent": "stills-generator",
  "action": "write_file",
  "path": "stills/scene_05.png",
  "prediction": "approve",
  "confidence": 0.94,
  "reasoning": "User requested scene 5 regeneration with reference"
}
```

### Decision Logic

| Confidence Range | Action |
|---|---|
| `> auto_approve_threshold` | Execute without asking. Log the prediction. |
| `< auto_deny_threshold` | Block. Notify user why. |
| Between thresholds | Surface to user. Log prediction AND user's decision. |

### Threshold Self-Tuning

Track prediction accuracy over a rolling window (last 100 predictions).

| Accuracy | Adjustment |
|---|---|
| > 95% | Lower `auto_approve_threshold` by 0.01 (more autonomy) |
| < 90% | Raise `auto_approve_threshold` by 0.02 (more caution) |
| 90%-95% | No change |

**Threshold clamps:** `auto_approve_threshold` is clamped to [0.50, 0.99]. The system is never fully autonomous and never fully gated.

### Default Thresholds

```yaml
# .plexi/agents/parallax/config.yaml
orchestrator:
  auto_approve_threshold: 0.85
  auto_deny_threshold: 0.20
  gate_threshold: 0.50
  prediction_window_size: 100
```

---

## 6. The Prediction Feedback Loop

Every prediction is logged as a JSONL entry:

```json
{
  "prediction_id": "pred_001",
  "timestamp": "2026-04-11T14:30:00Z",
  "agent": "stills-generator",
  "action": "write_file",
  "context_summary": "User requested scene 5 regen",
  "prediction": "approve",
  "confidence": 0.94,
  "actual": "approve",
  "correct": true,
  "orchestrator_version": "v3",
  "latency_ms": 240
}
```

**Storage:** `.plexi/agents/parallax/orchestrator/predictions.jsonl`

### Improvement Officer Review Cycle

The improvement officer periodically reviews prediction logs:

1. Identify patterns in wrong predictions (false approves, false denies).
2. Propose changes to the orchestrator's `system.md`.
3. Create a new versioned copy of the orchestrator (Section 7).
4. Run all test cases against the new version.
5. If quality holds or improves: promote the new version.
6. If regressions: stay on the old version, log why in `memory/`.

The review cycle can be triggered manually or run on a configurable schedule (default: after every 50 predictions).

---

## 7. Agent Versioning

Each agent supports version tracking via snapshots in `versions/`.

### Directory Layout

```
.plexi/agents/parallax/script-writer/
  versions/
    v1/
      system.md                  <- original system prompt
      memory/                    <- full memory snapshot at v1
      config.yaml                <- model, thresholds at v1
    v2/
      system.md                  <- refined prompt
      memory/                    <- compressed memory
      config.yaml
  current -> v2/                 <- symlink to active version
  test-cases/                    <- shared across versions
```

### Version Lifecycle

1. **Snapshot:** Before modifying an agent, copy the current state to `versions/vN/`.
2. **Modify:** Edit the live `system.md`, `memory/`, or `config.yaml`.
3. **Test:** Run all test cases (Section 8) against the modified agent.
4. **Promote:** If tests pass, create a new `versions/vN+1/` snapshot and update the `current` symlink.
5. **Rollback:** If tests fail, restore from `current` symlink target.

### Test Case Structure

```
test-cases/
  case-001/
    input/
      brief.md                   <- original user input
      reference.jpg              <- any reference files
    expected/
      output.md                  <- approved output
      manifest_state.yaml        <- manifest after successful run
      metrics.json               <- { cost_usd, tool_calls, duration_s, quality_score }
    runs/
      v1_run_001.json            <- v1's metrics on this case
      v2_run_001.json            <- v2's metrics on this case
```

`metrics.json` in `expected/` is the baseline. Runs are compared against it. A version passes a test case if:
- Output quality score >= baseline quality score (within configurable tolerance, default 5%)
- Cost does not exceed baseline cost by more than 20%
- No errors or unexpected tool calls

---

## 8. Test Case Auto-Capture

When a user approves an output, the system snapshots the full run as a test case.

### Capture Triggers

| Trigger | Condition |
|---|---|
| Explicit approval | User confirms a plan, accepts output, or says "looks good" |
| Implicit approval | User doesn't request changes within a configurable window (default: 10 minutes) |
| Quality gate | Evaluator scores above quality threshold AND user doesn't object |

### What Gets Captured

1. Input brief (the original user message or task description)
2. Reference files (images, docs, anything the user provided)
3. Approved output (the final artifact)
4. Manifest state (if applicable — project.yaml or equivalent after the run)
5. Metrics: cost in USD, tool call count, duration in seconds, quality score
6. Agent version that produced the output

### Storage

Test cases are stored in the producing agent's `test-cases/` directory, named `case-{NNN}/` with zero-padded sequential numbering.

Over time, this builds an organic regression suite from real production work. No manual test authoring required.

---

## 9. Memory Management

Agent memory files accumulate over time. The compression cycle prevents unbounded growth while preserving load-bearing context.

### Compression Cycle

1. Read current `memory/` contents (raw learnings, past decisions, user preferences).
2. Improvement officer distills: remove redundant entries, merge related learnings, tighten language.
3. Run all test cases against the agent with compressed memory.
4. If all test cases pass: compressed memory replaces original.
5. If any test case fails: identify which deleted memory entry was load-bearing, restore it, retry compression without that entry.
6. Log the compression: what was removed, what was kept, which test cases verified it.

### Schedule

Memory compression runs on a configurable schedule:
- Default: weekly
- Can be triggered on-demand via `plexi agent compress <agent-name>`
- Automatically triggered when memory exceeds a size threshold (default: 50KB)

### Compression Log Format

```yaml
# Appended to memory/compression-log.yaml
- timestamp: "2026-04-11T14:30:00Z"
  entries_before: 47
  entries_after: 31
  removed:
    - "Redundant: user prefers 16:9 (already in criteria.yaml)"
    - "Merged: three separate notes about pacing into one"
  kept_critical:
    - "Scene transitions must use crossfade (test-case-012 depends on this)"
  test_cases_verified: 15
  test_cases_passed: 15
```

---

## 10. Risk Scoring

Operations have risk scores (float, 0.0-1.0). These are starting points that self-tune based on observed approval patterns.

### Initial Risk Scores

| Operation | Initial Risk | Notes |
|---|---|---|
| `read_file` | 0.05 | Almost always safe |
| `list_dir` | 0.03 | Read-only |
| `write_file` (in project) | 0.25 | Creates or modifies |
| `write_file` (outside project) | 1.00 | Forbidden |
| `create_dir` | 0.15 | Low risk |
| `rename_file` | 0.40 | Moderate — can break references |
| `delete_file` | 0.65 | Destructive, recoverable |
| `delete_dir` | 0.80 | Destructive, harder to recover |
| `run_script` (approved list) | 0.35 | Known tool |
| `run_arbitrary_command` | 0.80 | Unknown behavior |
| `git_commit` | 0.30 | Reversible |
| `git_push` | 0.75 | Visible to others |
| `git_force_push` | 0.95 | Destructive to shared state |
| `network_request` (known API) | 0.20 | Known endpoint |
| `network_request` (unknown URL) | 0.70 | Unknown endpoint |
| `install_app` | 0.60 | Adds capabilities |
| `modify_permissions` | 0.85 | Security-sensitive |

### Forbidden Operations (risk = 1.0, always denied)

These operations are always denied regardless of trust score:
- Modify files outside the scoped directory
- Access system files (`/etc`, `/usr`, etc.)
- Disable logging or the audit trail
- Modify own trust score or risk scores
- Access other users' directories
- Send data to unregistered external endpoints (unless `network = true` in manifest AND user has approved the endpoint)

### Risk Self-Tuning

Risk scores drift based on observed approval patterns:
- Operation at risk 0.40 is consistently approved: risk drifts toward 0.30
- Operation at risk 0.40 is sometimes denied: risk drifts toward 0.50
- **Drift rate:** 0.005 per data point (very slow — requires ~50 approvals to shift by 0.25)
- **Risk floors:** Certain operations have minimum risk that can't be tuned below:

| Operation | Risk Floor |
|---|---|
| `delete_file` | 0.30 |
| `delete_dir` | 0.50 |
| `git_push` | 0.40 |
| `git_force_push` | 0.80 |
| `run_arbitrary_command` | 0.50 |
| `modify_permissions` | 0.60 |

---

## 11. Attention Levels for Background Jobs

When the orchestrator spawns background jobs, each job gets an attention level derived from the trust/risk calculation.

### Attention Levels

| Level | Condition | Behavior |
|---|---|---|
| **Autonomous** | Prediction confidence > `auto_approve_threshold` | Run silently, log results |
| **Notify** | Confidence between `gate_threshold` and `auto_approve_threshold` | Run to completion, show notification when done |
| **Gate** | Confidence < `gate_threshold` | Pause, show user what's about to happen, wait for approval |

### Dynamic Transitions

The attention level is NOT a fixed property of the job type. It's computed from current trust scores, risk scores, and prediction confidence. As trust grows, the same job type transitions:

```
Gate  -->  Notify  -->  Autonomous
(low trust)              (high trust)
```

A stills-generation job might start as Gate (first use), transition to Notify (after 10 successful runs), and eventually reach Autonomous (after 50+ successful runs with no user objections).

### Notification Format

For Notify-level jobs, the notification includes:
- What ran (agent, operation, target)
- Result summary (success/failure, output path, quality score if applicable)
- Option to review or undo

---

## 12. Multi-Agent Coordination Patterns

### Sequential Delegation

```
Orchestrator -> Agent A -> (result) -> Orchestrator -> Agent B -> (result) -> Orchestrator
```

Each step waits for the previous to complete. Used when Agent B needs Agent A's output.

**Example:** Script-writer produces a script, then storyboard-planner creates scenes from that script.

### Parallel Delegation

```
Orchestrator -> Agent A (background)
             -> Agent B (background)
             -> wait for both -> continue
```

Independent tasks run simultaneously. Used for stills generation across multiple scenes, or generating audio and visuals concurrently.

**Constraints:**
- The orchestrator must declare which outputs are independent before spawning parallel tasks.
- If any parallel task fails, the orchestrator decides whether to retry, skip, or abort the workflow. This is configured per-workflow in the orchestrator's `system.md`.

### Escalation

```
Agent A -> (confidence < threshold) -> Orchestrator -> Agent B (higher capability)
```

When a sub-agent isn't confident in its output, it escalates to the orchestrator. The orchestrator can:
1. Re-delegate to a more capable agent (e.g., swap from Haiku to Sonnet).
2. Add more context and retry with the same agent.
3. Surface to the user for guidance.

Escalation is logged in the prediction feedback loop with `"escalated": true`.

### Intervention

```
Agent A -> proposes action -> Orchestrator disagrees (prediction: deny) -> redirects Agent A
```

The orchestrator intercepts before the action reaches the user. The orchestrator provides corrective context to Agent A and asks it to revise. Only surfaces to the user if the orchestrator can't resolve the disagreement after one retry.

**When intervention triggers:**
- Agent proposes an action that contradicts the user's stated preferences (from memory).
- Agent proposes an action with risk score above the agent's trust level.
- Agent's proposed output conflicts with a previous step's output in the current workflow.

# Agent Replay & Testing Infrastructure

**Status:** Draft (vision spec)
**Last updated:** 2026-04-11
**Related specs:** [agent-orchestration.md](../subsystems/agent-orchestration.md), [app-infrastructure.md](../subsystems/app-infrastructure.md), [intelligence-protocol.md](../subsystems/intelligence-protocol.md) (deferred)

---

## 1. Vision

### What this is

A unified infrastructure layer for recording, replaying, branching, and evaluating agent runs across every Plexi app. The same primitives drive four distinct workflows:

1. **Debugging** — "why did this run go wrong?" Replay the exact sequence of prompts, tool calls, and responses.
2. **Iteration** — "does my refactored system prompt still work?" Fork a historical run from step N, swap one component, re-run forward, diff the outcomes.
3. **Regression** — "did the latest change break anything?" Run a curated suite of past runs against a modified component and flag deltas.
4. **Cost-aware evaluation** — "test as much as I can for $2." Framework selects the highest-fidelity tests it can afford under a budget, skipping ones it knows are meaningless at that fidelity.

### Why it matters

Plexi is an app platform where every app can host a non-trivial agent pipeline (Parallax video production, GitHub Issues triage, Focus Manager, Aquarium, etc.). Each pipeline is a sequence of LLM calls, tool invocations, and conditional branches. Today there is no shared way to:

- Reproduce a single bad run
- Ask "what would happen if I swapped the script-writer prompt at step 4?"
- Answer "which change caused the quality regression in the last 10 runs?"
- Know which tests are actually testing what they claim to test

Every app currently re-invents this infrastructure or skips it. The agent-orchestration spec already introduces `versions/`, `test-cases/`, and `predictions.jsonl` per agent — but each agent network owns that machinery privately. We need a Plexi-level primitive that every app can lean on the same way they lean on `cost_report` today.

### The motivating insight — "minify to zero"

The user's insight that shaped this spec:

> In stub mode, you could minify the prompt generator's system prompt to a zero-character string and it would have no impact on the output, because the image generator always returns the same stub. A test harness that reports "prompt generator passes" in stub mode is lying — it is physically incapable of detecting a regression in the prompt generator.

Put formally: a test is meaningful for a component C only if the fidelity of C's downstream consumers is high enough that changes in C's output produce observable changes in the final outcome. The framework must model this explicitly. If it can't, it should refuse to claim it has tested C.

This is the load-bearing idea of the whole spec. Everything else exists to make this checkable.

---

## 2. Existing Tools Survey

We are **not** building this from zero. Several ecosystems solve adjacent problems well. The goal is to understand the shape of each so Plexi can adopt patterns, integrate where sensible, and only build the thin slice that is genuinely Plexi-specific.

### 2.1 LangSmith (LangChain)

- **What it does well:** Tree-structured tracing with parent/child run linkage. Captures prompts, completions, tool calls, token counts, and latency per node. Curated dataset workflow — sample interesting production traces into a dataset and run evaluators against them. Mature LLM-as-judge + heuristic + human-annotation evaluator primitives.
- **What it misses:** Coupled to LangChain-shaped pipelines. "Replay" means re-running the same inputs against an evaluator, not forking a recorded run from step N with a swapped component. No notion of fidelity tiers or iterability gating. SaaS-first; the open-source surface is the SDK only.
- **Pattern to steal:** Tree-of-runs data model, root-and-children shape, "capture → sample → evaluate" lifecycle.

### 2.2 Langfuse (open-source)

- **What it does well:** OSS LLM engineering platform with tracing, prompt management, playground, datasets, and evals. Self-hostable. Integrates with OpenTelemetry, OpenAI SDK, LiteLLM. "Jump from a bad trace to the playground to iterate" loop is exactly the UX we want.
- **What it misses:** Playground iteration is single-prompt, not a full pipeline replay with a component swap. No fidelity/cost-budget selection. No "refuse to claim a test is meaningful" gate.
- **Pattern to steal:** Self-hostable posture, trace → playground bridge, the prompt-management versioning UI.

### 2.3 Braintrust

- **What it does well:** Side-by-side prompt comparison, experiments, regression detection in CI, "replay the exact sequence of decisions that led to a failure." Playground allows replaying and modifying previous interactions.
- **What it misses:** No automated multi-turn branching — teams manually prompt through conversations. No cost-budgeted eval selection. Closed platform.
- **Pattern to steal:** Side-by-side diff UX, experiment → comparison grid.

### 2.4 Helicone (OSS)

- **What it does well:** HTTP-level LLM observability via proxy. First-class caching (TTL, cache-key control, namespaces, multi-response cache) — this is effectively a low-level replay primitive. Sessions for tracking user journeys across multiple calls.
- **What it misses:** "Record a run, fork from step 7, swap system prompt, re-run" is out of scope. It is an observability gateway, not an iteration framework.
- **Pattern to steal:** HTTP-level cache as a replay primitive. The namespaced cache key with "exclude these JSON fields from the hash" is exactly how we should match stub responses.

### 2.5 Inspect (UK AISI)

- **What it does well:** Opinionated agent eval framework: `dataset → Task → Solver → Scorer`. First-class multi-turn agent workflows, tool calling, and sandboxed execution. Unified provider abstraction (OpenAI/Anthropic/Google/local). Designed for rigorous evals, not observability.
- **What it misses:** Evals are run-from-scratch — no "replay a captured production trace with one component swapped." No fidelity tiering.
- **Pattern to steal:** Task/Solver/Scorer primitive names. Sandboxed execution toolkit. Multi-provider abstraction. This is the closest existing thing to what Plexi should feel like at the eval layer.

### 2.6 Promptfoo

- **What it does well:** Declarative matrix testing (prompt × model × test case). Cost assertions as first-class — fail a CI run if cost exceeds a threshold. Easy CI integration via CLI.
- **What it misses:** Single-prompt oriented. Doesn't model multi-step agent pipelines. No replay/branching.
- **Pattern to steal:** `cost` as an assertion type. Declarative matrix syntax. Designed-for-CI CLI shape.

### 2.7 OpenLLMetry / Traceloop

- **What it does well:** OpenTelemetry semantic conventions for LLM spans (`gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.system_instructions`, token counts, model name, temperature). Already supported by downstream tools.
- **What it misses:** It is a wire format, not a framework. Defines *how* to emit spans, not what to do with them.
- **Pattern to steal:** **Use it as our on-wire format.** Plexi's run log should be OpenTelemetry-compatible LLM spans so existing observability tools can consume them. Don't reinvent span attribute names.

### 2.8 VCR.py / pytest-recording (the OG pattern)

- **What it does well:** Records HTTP interactions to a "cassette" file the first time, replays them on subsequent runs. Four recording modes: `once` (default), `new_episodes`, `none`, `all`. Tests become fast, deterministic, offline-capable.
- **What it misses:** HTTP-level only, no semantic understanding of LLM calls. Cassettes are test fixtures, not navigable first-class artifacts.
- **Pattern to steal:** **The cassette model is 90% of what we need.** Cassettes are the record/replay primitive Plexi should adopt wholesale. The four recording modes map almost one-to-one onto our fidelity spectrum (`none` = stub, `all` = pedal-to-metal). Record at the Plexi intelligence layer (or at the SDK wrapper layer for apps that make their own calls), store per-run, replay on demand.

### 2.9 Summary table

| Tool | Trace | Replay exact run | Fork/branch | Swap component | Cost budget | Open source | Fidelity tiers |
|---|---|---|---|---|---|---|---|
| LangSmith | yes | partial | no | no | no | SDK only | no |
| Langfuse | yes | yes (playground) | no | single prompt | no | **yes** | no |
| Braintrust | yes | yes | no | no | no | no | no |
| Helicone | yes | via cache | no | no | no | **yes** | no |
| Inspect | no (eval-first) | re-run | no | no | no | **yes** | no |
| Promptfoo | no | no | no | no | **yes** | **yes** | no |
| OpenLLMetry | yes (spans) | n/a | n/a | n/a | n/a | **yes** | n/a |
| VCR.py | yes (http) | **yes** | no | no | n/a | **yes** | partial |
| **Plexi target** | **yes** | **yes** | **yes** | **yes** | **yes** | **yes** | **yes** |

### 2.10 Verdict — build-vs-buy

Plexi should **not** build its own tracing format, provider abstraction, or eval scorer library. Those exist and are good.

Plexi **must** build:

1. The fidelity spectrum and the iterability-aware gate (no one has this).
2. The cost-budgeted test selection (Promptfoo has it per-assertion, but not as a budgeted selection algorithm).
3. The fork-from-step-N-with-component-swap primitive on top of a recorded run.
4. The Plexi-native surfacing — a replay browser app that consumes the data.

Everything else should be composed from existing patterns: OpenLLMetry spans as the wire format, VCR.py-style cassettes as the replay artifact, Inspect's `Task/Solver/Scorer` vocabulary at the eval layer, Langfuse-style trace-to-playground bridging at the UX layer.

---

## 3. The Fidelity Spectrum

Four named fidelity modes. Every test run declares a mode. Every component declares the minimum mode required to meaningfully test it. The framework reconciles.

### 3.1 Mode definitions

| Mode | Intent | LLM behavior | Image gen | Network tools | Filesystem | Typical cost per run |
|---|---|---|---|---|---|---|
| `stub` | Shape/wiring check, fastest possible | Deterministic canned response per (role, step) | Fixed fixture image | Canned fixtures | Real (scope-locked) | $0.00 |
| `cheapest` | End-to-end semantic check, minimum spend | Cheapest real model (e.g. `claude-haiku`, `gemini-flash`) at low max_tokens | Cheapest real gen (e.g. FLUX Schnell) | Real, rate-limited | Real | ~$0.01–$0.10 |
| `default` | Production models, normal cost | Production tier (`claude-sonnet-4-6`, etc.) at normal settings | Production quality | Real | Real | ~$0.10–$2.00 |
| `pedal` | Best available, full quality | Top-tier model (`claude-opus-4-6`) with large context and max_tokens | Best image model | Real, longer timeouts | Real | $1.00–$20+ |

The thresholds are not magic numbers — they're resolved from `~/.plexi/config.toml` under a new `[replay.fidelity]` section:

```toml
[replay.fidelity.stub]
# Stub mode uses no real calls. Responses are replayed from cassettes
# or generated by the stub registry.
llm_tier = "stub"
image_tier = "stub"

[replay.fidelity.cheapest]
llm_tier = "low"          # resolves to low_model from [intelligence]
image_tier = "speed"
max_tokens = 1024

[replay.fidelity.default]
llm_tier = "medium"
image_tier = "quality"

[replay.fidelity.pedal]
llm_tier = "high"
image_tier = "quality"
max_tokens = 8192
```

`llm_tier = "stub"` is a synthetic tier the intelligence layer understands: return the cassette response if one exists, else return the app-registered stub, else error out.

### 3.2 Why a spectrum, not a toggle

Every existing tool we surveyed treats fidelity as on/off (record/replay, real/mock). This collapses four real questions into one:

- **Can I run this at all without spending money?** (stub)
- **Can I run the full pipeline for pennies to catch wiring bugs?** (cheapest)
- **Am I reproducing production?** (default)
- **Am I measuring the ceiling of what this component can do?** (pedal)

A test run at `cheapest` is a legitimate regression test for wiring, control flow, and schema conformance — but it is not a valid regression test for a component whose job is "produce a subjectively better caption than last week's." Those two facts both need to be expressible.

### 3.3 Worked example — Parallax stills pipeline

A run moves through: `script-writer → storyboard-planner → prompt-generator → image-generator → evaluator`.

| Component | Can test at `stub`? | At `cheapest`? | At `default`? | At `pedal`? | Why |
|---|---|---|---|---|---|
| script-writer | yes | yes | yes | yes | Output is text consumed by another LLM — any real model produces a testable signal downstream. |
| storyboard-planner | yes | yes | yes | yes | Same shape as script-writer. |
| prompt-generator | **no** | yes | yes | yes | Output is only observable via the image generator. In stub mode the image is canned, so prompt changes have zero downstream effect. Framework must refuse. |
| image-generator | **no** | yes | yes | yes | Stub mode returns a fixed image by construction. |
| evaluator | yes (with fixed inputs) | yes | yes | yes | Evaluator reads the full run state; you can exercise its scoring logic with canned upstream output. |
| end-to-end quality | no | no | yes | yes | "Did this produce a good video?" requires production models. `cheapest` models can wire the pipeline but not validate quality. |

This is the matrix the framework has to reason about. It can't be general — it's per-app and must be declared.

---

## 4. Iterability Manifest

Each app ships a declaration of what components exist and what fidelity each needs to be meaningfully tested. This lives next to the existing `manifest.toml` as `replay.toml` (or inside a `[replay]` section of the main manifest — see open question §12.1).

### 4.1 Schema

```toml
# replay.toml — declares replay/test metadata for this app.

[replay]
schema_version = "1"
app_id = "parallax"

# Components the app exposes for independent iteration.
# Each key is a component name used by the SDK when emitting spans.

[replay.components.script_writer]
kind = "llm"
description = "Generates the script from a user brief."
requires_mode = "cheapest"   # stub | cheapest | default | pedal
stub = { type = "file", path = "replay/stubs/script_writer.txt" }
# 'stub' is what the framework returns when this component runs in stub mode.

[replay.components.storyboard_planner]
kind = "llm"
description = "Decomposes a script into scenes."
requires_mode = "cheapest"
stub = { type = "file", path = "replay/stubs/storyboard.json" }

[replay.components.prompt_generator]
kind = "llm"
description = "Writes image prompts from storyboard scenes."
# CRITICAL: prompt generator changes are invisible in stub mode
# because the image is canned. Framework must refuse to run a
# prompt_generator iteration test at stub fidelity.
requires_mode = "cheapest"
downstream_consumers = ["image_generator"]
stub = { type = "file", path = "replay/stubs/image_prompt.txt" }

[replay.components.image_generator]
kind = "image_gen"
description = "Generates the still image."
requires_mode = "cheapest"
stub = { type = "file", path = "replay/stubs/still.png" }

[replay.components.evaluator]
kind = "llm"
description = "Scores the final output against the brief."
requires_mode = "stub"        # evaluator logic can be exercised with canned inputs
stub = { type = "file", path = "replay/stubs/evaluator_score.json" }

# End-to-end qualities the app wants tracked as a single cross-component score.

[[replay.suites]]
name = "quality_smoke"
description = "End-to-end quality of the final video."
requires_mode = "default"
scorer = "llm_as_judge"
cases = "test-cases/**/case-*"

[[replay.suites]]
name = "wiring_smoke"
description = "Pipeline shape, schema conformance, tool availability."
requires_mode = "stub"
scorer = "heuristic"
cases = "test-cases/**/case-*"
```

### 4.2 `requires_mode` semantics

`requires_mode` is the **minimum** fidelity at which a change to this component can produce an observable signal. It is a lower bound, not an equality:

- If the test harness runs the suite at `default` and `prompt_generator.requires_mode = "cheapest"`, the test is valid — `default` ≥ `cheapest`.
- If the harness runs at `stub` and `prompt_generator.requires_mode = "cheapest"`, the test is **not meaningful for prompt_generator** and the harness must either:
  1. **Refuse** to include `prompt_generator` in the scored components for this run (default).
  2. **Upgrade** just that component's call to `cheapest` fidelity (`--auto-upgrade`).
  3. **Run and flag** as a false-positive-prone result (`--allow-meaningless`, explicit opt-in, emits a loud warning in the report).

The default is refuse. Silence is a lie we're specifically trying to avoid.

### 4.3 `kind` field — why not just `llm`

`kind` tells the framework how to substitute the component in stub mode and how to attribute cost:

| kind | Stub strategy | Cost model |
|---|---|---|
| `llm` | Return cassette or file stub; log fake token counts | Tokens × rate |
| `image_gen` | Return cassette or image file; log fake generation | Per-image rate |
| `tool` | Return cassette or JSON stub | Free (local) |
| `network` | Return cassette or JSON stub | Free (stubbed), rate-limited (live) |
| `code` | No stub — always runs (determinism comes from fixed inputs) | Free |

### 4.4 Stub registration from the SDK

Apps can also register stubs at runtime via the SDK (§10), for cases where the stub is dynamic (e.g., needs to mirror the input shape):

```python
@app.on_stub("prompt_generator")
def stub_prompt_generator(inputs):
    return f"[STUB prompt for scene {inputs['scene_index']}]"
```

Runtime stubs override manifest stubs when present. The harness records this in the run metadata so a replay can reconstruct which stub source was active.

---

## 5. Replay Format — What a Recorded Run Looks Like

### 5.1 Storage location

```
~/.plexi-alpha/replay/
  runs/
    2026-04-11T14-32-01_parallax_7c3a/
      run.json              <- run metadata (see 5.2)
      spans.jsonl           <- ordered OpenTelemetry LLM spans (5.3)
      cassettes/
        llm_001.json        <- captured LLM request/response (5.4)
        llm_002.json
        img_001.json        <- captured image_gen request/response
        tool_001.json       <- captured tool call
      artifacts/
        stills/scene_01.png <- any files the run produced
        stills/scene_02.png
      inputs/
        brief.md            <- the initial input to the run
      outputs/
        final.mp4
      manifest_snapshot.toml <- copy of app's manifest at time of run
      replay_snapshot.toml   <- copy of replay.toml at time of run
      costs.jsonl            <- subset of costs.jsonl entries tagged to this run
```

### 5.2 `run.json`

```json
{
  "schema_version": "1",
  "run_id": "2026-04-11T14-32-01_parallax_7c3a",
  "app_id": "parallax",
  "app_version": "0.4.2",
  "started_at": "2026-04-11T14:32:01Z",
  "ended_at": "2026-04-11T14:34:18Z",
  "fidelity_mode": "default",
  "fidelity_overrides": {
    "evaluator": "cheapest"
  },
  "trigger": {
    "kind": "user",
    "directory": "/Users/ian/projects/client-a",
    "user_message": "Make a 30s ad for the marble pouch"
  },
  "parent_run_id": null,
  "branch_point": null,
  "total_cost_usd": 0.83,
  "components_exercised": [
    "script_writer",
    "storyboard_planner",
    "prompt_generator",
    "image_generator",
    "evaluator"
  ],
  "outcome": {
    "status": "completed",
    "scores": { "quality_smoke": 0.78 }
  },
  "env": {
    "plexi_version": "0.8.1-alpha",
    "intelligence": {
      "low_model": "claude-haiku-4-5",
      "medium_model": "claude-sonnet-4-6",
      "high_model": "claude-opus-4-6"
    }
  }
}
```

### 5.3 `spans.jsonl`

One OpenTelemetry-compatible LLM span per line, in the OpenLLMetry `gen_ai.*` semantic convention. This is the authoritative ordered event stream.

```json
{"span_id":"s001","parent_span_id":null,"name":"parallax.run","start":"...","end":"...","attrs":{"app_id":"parallax","run_id":"...","component":null}}
{"span_id":"s002","parent_span_id":"s001","name":"script_writer.invoke","start":"...","end":"...","attrs":{"component":"script_writer","gen_ai.system":"anthropic","gen_ai.request.model":"claude-sonnet-4-6","gen_ai.response.model":"claude-sonnet-4-6","gen_ai.usage.input_tokens":1200,"gen_ai.usage.output_tokens":450,"gen_ai.input.messages":[...],"gen_ai.output.messages":[...],"plexi.cassette_ref":"cassettes/llm_001.json","plexi.cost_usd":0.012}}
{"span_id":"s003","parent_span_id":"s001","name":"storyboard_planner.invoke","..."}
...
```

Key Plexi-specific attributes on top of `gen_ai.*`:

| Attribute | Purpose |
|---|---|
| `plexi.run_id` | Join key to `run.json` |
| `plexi.component` | Maps to `replay.components.*` in the manifest |
| `plexi.cassette_ref` | Relative path to the captured request/response fixture |
| `plexi.cost_usd` | Real cost charged to this span |
| `plexi.fidelity_mode` | Mode this span was executed under |
| `plexi.branch_of` | If this span is part of a branched re-run, the source span_id |

Using OpenLLMetry means we can point Grafana/Tempo/Honeycomb/Langfuse at the same `spans.jsonl` and get observability for free.

### 5.4 Cassette format

One cassette per external call. Cassettes are content-addressed for dedup across runs.

```json
{
  "kind": "llm",
  "request_hash": "sha256:...",
  "request": {
    "provider": "anthropic",
    "model": "claude-sonnet-4-6",
    "system": "You are a video script writer.",
    "messages": [{"role":"user","content":"Make a 30s ad..."}],
    "max_tokens": 4096,
    "tools": []
  },
  "response": {
    "text": "Scene 1. The pouch sits on marble...",
    "input_tokens": 1200,
    "output_tokens": 450,
    "stop_reason": "end_turn"
  },
  "metadata": {
    "recorded_at": "2026-04-11T14:32:03Z",
    "latency_ms": 2140,
    "cost_usd": 0.012,
    "fidelity_mode": "default"
  }
}
```

The `request_hash` excludes fields that shouldn't affect the cache key (request IDs, timestamps, trace IDs). This mirrors Helicone's "exclude these JSON keys from cache key" feature and VCR.py's custom matcher support.

### 5.5 Why JSONL + per-run directory, not SQLite

Considered. Rejected for MVP because:

1. **Grep-ability.** `rg 'prompt_generator' ~/.plexi-alpha/replay/runs/*/spans.jsonl` is the first thing anyone will want to do. SQLite kills that.
2. **Per-run isolation.** A run is a natural filesystem unit — you can `rm -rf` it, zip it, email it, commit it as a regression fixture, copy it to another machine.
3. **Append-only semantics.** JSONL matches the `costs.jsonl` / `predictions.jsonl` pattern Plexi already uses.
4. **No query layer yet.** When we need it, add a SQLite index on top as a view — don't move the source of truth.

When the run count exceeds ~10,000 we will likely add an index file (`~/.plexi-alpha/replay/index.sqlite`) that the replay browser app reads for fast filtering. The runs directory remains authoritative.

---

## 6. Branching Model — "Fork from step N, swap component, re-run"

### 6.1 Concrete operation

```
plexi replay fork <run_id> --from-span <span_id> --swap <component>=<source>
```

Example:

```
plexi replay fork 2026-04-11T14-32-01_parallax_7c3a \
  --from-span s004 \
  --swap prompt_generator=path/to/new_system.md \
  --fidelity default
```

This produces a new run `2026-04-11T15-02-11_parallax_7c3a_fork_s004_b2f1` with:

- `parent_run_id = "2026-04-11T14-32-01_parallax_7c3a"`
- `branch_point = "s004"`
- Cassettes for spans **before** `s004` are replayed verbatim from the parent run.
- Span `s004` onward is **re-executed** against the swapped component, hitting real models (or cached cassettes at the swapped fidelity).

### 6.2 What "swap" means for each kind

| Swap target | Meaning |
|---|---|
| `--swap prompt_generator=./new_system.md` | Replace the system prompt file for this component, re-run from the fork point. |
| `--swap prompt_generator=@version:v3` | Use the `versions/v3/system.md` from the agent-orchestration spec. |
| `--swap script_writer.model=claude-opus-4-6` | Override the resolved model for this component only. |
| `--swap evaluator.code=path/to/new_scorer.py` | Replace a code-kind component. |
| `--swap image_generator=stub` | Force this component to its registered stub for the re-run. |

The fork operation is idempotent as long as the upstream cassettes and the swapped component are stable. Non-determinism at the re-executed portion (temperature > 0, real image gen, real tool calls) is an inherent part of the test — the framework records the new spans and the user compares.

### 6.3 Determinism posture

We take a **pragmatic** stance on determinism, not a pure one:

- **Upstream of the fork point:** fully deterministic by cassette replay.
- **At and after the fork point:** non-deterministic by default. Temperature, seeds, and tool side effects are as-configured by the app.
- **`--seed <int>`:** best-effort — passed to providers that support it (OpenAI, some local models). Anthropic does not expose a seed, so this is documented as "no guarantee."
- **`--runs <N>`:** run the fork N times and aggregate. This is the honest way to deal with non-determinism. Default is 1 for interactive use, higher for scheduled suites.

We do **not** try to pin model snapshots. Anthropic and Google do not expose reliable immutable snapshot IDs across all models. The best we can do is record `gen_ai.response.model` in the span and let the user read it. Model drift between a parent run and its fork is a real effect and should be visible, not hidden.

### 6.4 Diff output

After a fork completes, `plexi replay diff <parent_run_id> <fork_run_id>` produces:

```
diff parallax 2026-04-11T14-32-01 → 2026-04-11T15-02-11
  branch point: s004 (prompt_generator.invoke)
  swapped: prompt_generator.system_prompt

  s004  prompt_generator.invoke         cost +$0.004  tokens 820 → 1120
  s005  image_generator.invoke          cost +$0.040  prompt changed
  s006  evaluator.invoke                score 0.78 → 0.84  (+0.06)

  artifacts/stills/scene_01.png         changed (hash mismatch)
  artifacts/stills/scene_02.png         changed
  outputs/final.mp4                     changed

  suite quality_smoke: 0.78 → 0.84  PASS (within +20% cost envelope)
```

---

## 7. Cost Accounting

### 7.1 Three phases

1. **Predict** (before the run). For each span the framework expects to execute, estimate token count from prior runs of the same component at the same fidelity, multiply by the resolved model's rate, sum. Falls back to a conservative default if no priors exist.
2. **Track** (during the run). As each span completes, append real cost to `costs.jsonl` (reusing the existing cost-tracker path from intelligence-protocol) and tag it with `plexi.run_id`. Compare running total to the predicted total — if running total exceeds prediction by more than `[replay.cost].blow_out_ratio` (default 2.0), surface a warning.
3. **Report** (after the run). Final cost rolls into `run.json.total_cost_usd` and a breakdown goes into `costs.jsonl` entries. The replay browser shows per-component and per-run aggregation.

### 7.2 Budget-directed test selection

The novel piece. When the user runs:

```
plexi replay test --budget 2.00 --app parallax
```

The framework:

1. Loads all components and suites from `replay.toml`.
2. For each (component, suite, mode) triple that is meaningful under the constraint `mode ≥ component.requires_mode`:
   - Estimate cost from prior runs.
   - Estimate signal value (see 7.3).
3. Solves a 0-1 knapsack: maximize total signal value subject to total estimated cost ≤ budget.
4. Runs the selected tests.
5. Reports both what was run and what was skipped due to budget.

Output:

```
Budget: $2.00
Selected 7 tests ($1.83 est):
  [stub]     wiring_smoke (5 cases, $0.00) — 12 components
  [cheapest] prompt_generator iteration (3 cases, $0.09) — 3 cases
  [default]  quality_smoke (2 cases, $1.74) — 2 cases

Skipped 4 tests (would exceed budget):
  [pedal]    quality_ceiling (2 cases, $12.40) — over budget
  [default]  prompt_generator iteration (3 cases, $0.54) — lower value than selected
```

### 7.3 Signal value heuristic (MVP)

`signal = priority_weight × recency_factor × fidelity_bonus × volatility`

- `priority_weight`: from the suite declaration (`priority = "high" | "medium" | "low"` in `replay.toml`).
- `recency_factor`: higher if the component was recently modified (`git log` on the system prompt / source file).
- `fidelity_bonus`: `default > cheapest > stub`. Higher fidelity gives more trustworthy signal per dollar.
- `volatility`: higher if this component has had score variance across recent runs (flaky = more important to test).

This is a heuristic, not a learned model. It will be wrong in interesting ways. The user should be able to inspect and override the ranking — `--explain-selection` dumps the ranked list with scores.

### 7.4 Cost model table (MVP)

Per-provider rates live in `~/.plexi/config.toml` under `[replay.rates]`:

```toml
[replay.rates.anthropic]
"claude-haiku-4-5"  = { input = 0.25, output = 1.25, unit = "per_million_tokens" }
"claude-sonnet-4-6" = { input = 3.00, output = 15.00, unit = "per_million_tokens" }
"claude-opus-4-6"   = { input = 15.00, output = 75.00, unit = "per_million_tokens" }

[replay.rates.google]
"imagen-3" = { per_image = 0.04 }
"gemini-2.0-flash" = { input = 0.075, output = 0.30, unit = "per_million_tokens", per_image = 0.002 }
```

Rates must be explicit. Missing a rate for a model the framework sees is a loud error — we do not invent fallbacks, per the Configuration Philosophy in `CLAUDE.md`.

---

## 8. Aggregated Analysis — "Insights for agent runs"

The same shape as the existing video-production insights reports but scoped to agent runs.

### 8.1 Command

```
plexi replay insights --app parallax --last 100
```

### 8.2 What it does

1. Load the most recent N runs for the app.
2. Join on spans, costs, and outcome scores.
3. Compute roll-ups: cost trend, quality trend, per-component latency distribution, most common failure modes (from `outcome.status = "failed"` runs).
4. Pass the aggregate into an LLM (tier `medium`) with a fixed analysis prompt: "find patterns across these runs, suggest three things to investigate."
5. Emit an HTML report to `~/.plexi-alpha/replay/reports/insights_YYYY-MM-DD.html`.

### 8.3 Output shape

```
Parallax — Last 100 runs (2026-03-24 → 2026-04-11)
  Total cost: $47.32
  Quality mean: 0.81 (σ 0.08)

  Cost trend: +23% over last 30 runs  [chart]
  Quality trend: -0.04 over last 30 runs  [chart]

  Hotspots:
    1. image_generator cost spiked +40% after 2026-04-05 model switch
    2. evaluator latency p95 doubled after v3 prompt change
    3. prompt_generator flake rate: 12% (3 runs scored <0.5)

  LLM-suggested investigations:
    1. Revisit the 2026-04-05 image_generator default. Cost increase is not
       tracked by quality increase. Candidate for rollback to v2 config.
    2. Evaluator v3 prompt is longer than v2. Check if tightening recovers latency.
    3. Capture the 3 prompt_generator flakes as test cases and run at default
       fidelity against v2 and v3 to localize.
```

### 8.4 Relationship to `predictions.jsonl`

The agent-orchestration spec already captures a per-orchestrator `predictions.jsonl` for trust calibration. Replay insights reads that file too and folds prediction accuracy into the report. No duplication — predictions stay where they are, replay insights is a read-only consumer.

---

## 9. App vs Core Split

This is a real decision, not a rhetorical one. The user's own framing (from the session): "should this be a Plexi app or core Plexi infrastructure?"

### 9.1 Recommendation

**Split it in two.** Core owns the data and the primitives; an app owns the UX.

| Layer | Lives in | What it is |
|---|---|---|
| **Capture** | Core (Rust) | Spans and cassettes are written by the intelligence layer (and by SDK wrappers for apps that call LLMs directly). Apps cannot opt out — if an app uses the intelligence protocol or the SDK intelligence helpers, it is recorded. |
| **Storage** | Core (filesystem) | `~/.plexi-alpha/replay/runs/` is a core-owned directory. Schema-versioned. |
| **Replay execution** | Core (Rust) | The fork/re-run engine is core. It needs access to the intelligence layer, the cassette store, and the app process spawner. This cannot live in an app. |
| **CLI** | Core | `plexi replay fork|diff|test|insights` commands. Thin wrappers over core APIs. |
| **Fidelity spectrum** | Core | The `[replay.fidelity]` config and the tier resolver are core. Apps read them via the SDK. |
| **Iterability gating** | Core | The "refuse to run meaningless tests" logic is core policy. Apps declare; core decides. |
| **Replay browser UI** | **App** (Plexi app) | Browse runs, diff runs, trigger forks, view insights. A normal Plexi app that consumes core APIs via a new `replay.*` API request family. |
| **Insights analysis** | App (Plexi app) | LLM-driven pattern detection is app-level. The app reads `spans.jsonl` and `run.json` files (with `filesystem.read` permission) and drives the analysis. |

### 9.2 Why this split

- **Capture must be core** because app authors should not have to re-implement it and should not be able to opt out of audit-level recording of agent runs. This is the same reason `cost_report` is mediated by core.
- **Replay execution must be core** because it has to spawn app processes, inject cassettes at the intelligence layer, and gate budget. An app cannot spawn another app.
- **UI should be an app** because the UI is unopinionated — some users will want a list, some a graph, some a diff view, some an LLM chat over runs. Making the UI an app means alternatives can coexist. It also eats our own dogfood: if we can't build the replay browser as a Plexi app, the app protocol is missing something.
- **Insights should be an app** because it is the most opinionated surface and will change fastest. Keeping it out of core lets it iterate without core releases.

### 9.3 New API request family

Added to the app protocol (§3 of app-infrastructure.md), mediated by capability:

| Request | Description | Capability |
|---|---|---|
| `ReplayListRuns { app_id?, since?, limit? }` | List recorded runs, optionally filtered. | `replay.read` |
| `ReplayGetRun { run_id }` | Return `run.json`, span count, cost, outcome. | `replay.read` |
| `ReplayGetSpans { run_id }` | Stream spans as draw-protocol events or return as array. | `replay.read` |
| `ReplayFork { run_id, from_span, swaps, fidelity }` | Start a fork run. Async. | `replay.write` |
| `ReplayTest { app_id, budget_usd, mode }` | Start a budgeted test run. Async. | `replay.write` |
| `ReplayDiff { run_a, run_b }` | Compute and return a structured diff. | `replay.read` |

New permissions: `replay.read`, `replay.write`. Default off. The built-in replay-browser app ships with both. Third-party apps can request `replay.read` to build alternative UIs.

---

## 10. SDK Additions

### 10.1 Python (`plexi_sdk.py`)

Add a `ReplayContext` helper that wraps LLM calls and emits spans. Minimal change to the existing cost_report flow — the span is emitted as a new draw-protocol event, not a new subprocess protocol.

```python
from plexi_sdk import App, record

app = App()

@app.on_command
def on_command(text, emit):
    with record.run("generate_script", inputs={"brief": text}) as run:
        with run.component("script_writer", kind="llm") as c:
            reply = call_anthropic(system=SCRIPT_SYS, messages=[...])
            c.record(
                model="claude-sonnet-4-6",
                input_messages=[...],
                output=reply.text,
                input_tokens=reply.input_tokens,
                output_tokens=reply.output_tokens,
                cost_usd=0.012,
            )

        with run.component("prompt_generator", kind="llm") as c:
            # If we're in stub mode and this component's requires_mode is cheapest,
            # Plexi will raise ReplayMeaninglessError here unless we explicitly pass
            # allow_meaningless=True. By default, the app fails fast so we don't
            # silently generate bogus test results.
            reply = call_anthropic(...)
            c.record(...)

        # ... more components
```

Under the hood, `record.run(...)` sends a `replay_run_start` event to Plexi, `run.component(...)` emits `replay_component_start` / `replay_component_end`, and `c.record(...)` emits the span payload. Plexi writes the run directory and cassettes from the core side.

Stub registration:

```python
@app.on_stub("prompt_generator")
def stub_prompt_generator(inputs):
    return {"prompt": f"[STUB prompt scene {inputs.get('scene_index', 0)}]"}
```

`plexi_test.py` gains a new mode:

```python
from plexi_test import AppTestHarness, fidelity

with AppTestHarness("path/to/app.py", fidelity=fidelity.STUB) as h:
    h.send_init()
    run_id = h.run_scenario("generate_script", inputs={"brief": "..."})
    spans = h.get_run_spans(run_id)
    h.assert_component_ran("script_writer", spans)
    h.assert_component_meaningful("prompt_generator", spans)  # raises in stub mode
```

### 10.2 Rust SDK

Parallel API: `plexi_sdk::replay::RunBuilder`, `ComponentRecorder`. Spans serialized as JSON, written to stdout through the existing draw-protocol event stream.

```rust
let mut run = replay::run("generate_script", json!({"brief": brief}))?;

{
    let mut c = run.component("script_writer", Kind::Llm)?;
    let reply = call_anthropic(&system, &messages).await?;
    c.record(ComponentRecord {
        model: "claude-sonnet-4-6".into(),
        input_tokens: reply.input_tokens,
        output_tokens: reply.output_tokens,
        cost_usd: 0.012,
        ..Default::default()
    })?;
}
```

Rust apps already write spans via the same path as Python — this is just syntactic sugar over the same draw-protocol events.

### 10.3 New protocol events

| Event | Direction | Payload |
|---|---|---|
| `replay_run_start` | App → Plexi | `{ name, inputs, parent_run_id?, fidelity_mode? }` |
| `replay_run_end` | App → Plexi | `{ run_id, status, scores? }` |
| `replay_component_start` | App → Plexi | `{ run_id, component, kind }` |
| `replay_component_record` | App → Plexi | `{ run_id, component, span_payload }` (OpenLLMetry shape) |
| `replay_component_end` | App → Plexi | `{ run_id, component }` |
| `replay_inject_cassette` | Plexi → App | `{ component, response }` — used during replay to tell the app to return this canned value instead of making a real call |

`replay_inject_cassette` is the key thing. During a replay, when the SDK is about to make a real LLM call inside a component that has a recorded cassette, it consults Plexi first, gets back the canned response, and returns it without calling the provider. This is the VCR.py pattern, moved to our SDK level instead of HTTP level. It works even for apps that call providers directly (which is the current architecture — see intelligence-protocol.md's deferred status).

---

## 11. Implementation Phases

### Phase 0 — Foundations (merge what already exists)

- Unify `costs.jsonl` ingestion with the replay layer. A run_id tag on cost events is the minimal join key.
- Define OpenLLMetry semantic conventions and the Plexi extension attributes (`plexi.run_id`, `plexi.component`, etc.).
- Reuse the existing agent-orchestration `test-cases/` layout for suite inputs.

### Phase 1 — MVP (record + replay only)

**Ship criteria:** you can record a Parallax run and replay it at stub fidelity.

- `~/.plexi-alpha/replay/runs/` directory scheme.
- `run.json`, `spans.jsonl`, `cassettes/` writer in core.
- Python SDK `record.run()` / `record.component()` helpers.
- Stub mode: `replay_inject_cassette` path; manifest-declared file stubs.
- `plexi replay list` and `plexi replay show <run_id>` CLI commands.
- No UI, no fork, no budget.

### Phase 2 — Fork + diff

**Ship criteria:** you can fork from step N with a swapped component and see a diff.

- `plexi replay fork` CLI.
- `plexi replay diff` CLI.
- The `--swap` operators for `system_prompt`, `model`, and `version`.
- Cassette replay from parent run up to the fork point.

### Phase 3 — Iterability gate

**Ship criteria:** `plexi replay test --mode stub --include prompt_generator` refuses to run with a clear error.

- `replay.toml` schema and parser.
- The `requires_mode` check in the replay runner.
- `--auto-upgrade` and `--allow-meaningless` flags.

### Phase 4 — Fidelity spectrum + cost budget

**Ship criteria:** `plexi replay test --budget 2.00` selects and runs a meaningful subset.

- `[replay.fidelity]` and `[replay.rates]` config.
- Cost prediction from prior runs.
- Knapsack-based test selection.
- `--explain-selection` output.

### Phase 5 — Replay browser app

**Ship criteria:** you can browse runs, trigger forks, and view diffs from inside Plexi.

- Built-in app at `~/.plexi-alpha/apps/replay-browser/`.
- `replay.*` API request family with `replay.read`/`replay.write` capabilities.
- List/detail/diff views. Fork trigger. Insights view (read-only in this phase).

### Phase 6 — Insights

**Ship criteria:** `plexi replay insights --last 100` produces a useful HTML report.

- Aggregation pipeline (cost/quality/latency trends).
- LLM-driven pattern suggestion (tier `medium`).
- HTML report templating (reuse video-production insights template where it fits).
- Timestamped copy pattern (`insights_YYYY-MM-DD.html`).

### Phase 7 — Sharing and import/export

**Ship criteria:** you can export a run as a regression fixture and share it.

- `plexi replay export <run_id> --out file.plexirun` → tarball with schema version.
- `plexi replay import file.plexirun`.
- Optional cassette redaction (strip secrets/PII before share).

### What we do **not** build

- Our own tracing backend (use OpenLLMetry-compatible spans, let users point existing tools at them).
- Our own provider abstraction (apps call providers directly per current architecture; intelligence-protocol is deferred).
- Our own HTTP proxy (Helicone exists; apps can opt into it independently).
- A hosted service.
- A training-data export pipeline. If someone wants that, they can read `spans.jsonl` themselves.

---

## 12. Open Questions

### 12.1 One manifest or two?

`replay.toml` as a separate file, or a `[replay]` section in `manifest.toml`? Argument for separate: can be large, authored by different people (test eng vs app dev), and the app manifest is already getting crowded. Argument for one file: fewer files to discover, one schema to validate, atomic with app version. **Leaning: separate file, referenced from `manifest.toml`** via `replay_manifest = "replay.toml"`.

### 12.2 Stub determinism in stub-mode chains

If stub A feeds stub B, and stub B's stub function takes A's output as an input, we get deterministic-but-synthetic data flow. Is that ever useful, or do we hard-require canned stubs at every component? **Current take:** allow both. Function stubs can read upstream output via a `StubContext` argument. Document loudly that this is for wiring validation, not semantic validation.

### 12.3 Cassette matching for non-deterministic inputs

If a test case feeds the app a timestamp or a uuid, the cassette hash won't match on replay. VCR.py solves this with custom matchers. We will need the same. **Open:** is this a per-app matcher declaration in `replay.toml`, or a general field-ignore list at the Plexi level?

### 12.4 Cross-run cassette dedup

Content-addressed cassettes can be shared across runs. Worth it? **Leaning yes** — a global `cassettes/` pool with symlinks from each run directory. Saves disk, enables "how many runs would this cassette change affect" queries. Defer to Phase 1.5.

### 12.5 Interaction with agent-orchestration `versions/`

The orchestration spec already has per-agent `versions/vN/` snapshots and `test-cases/`. The replay infrastructure could reuse them directly or maintain parallel storage. **Strong preference:** reuse. `--swap agent=@version:v3` should resolve to the orchestration spec's `versions/` directory. The replay format's `test-cases/` field should be a glob against orchestration's layout. Validate this works in Phase 1 with Parallax.

### 12.6 What about tool calls that mutate state

Cassette-replaying an LLM call is safe. Cassette-replaying `write_file("/tmp/output.png", ...)` is safe as long as we record what was written. Cassette-replaying `run_command("curl -X POST https://...")` is **not** safe — the side effect has already happened. For MVP, tools flagged `side_effect = true` in the app's tool manifest are not stub-able and will either run for real or cause the replay to fail loudly. This is annoying but correct.

### 12.7 Signal value heuristic is going to be wrong

The knapsack signal function in §7.3 is guessing. How do we feed back "you picked the wrong tests" into future selections? **Deferred.** For MVP, make it inspectable (`--explain-selection`) and overridable (`--pin <suite>`). Learning comes later, after we have enough run history to have real signal.

### 12.8 Streaming responses

LLM streaming is the default in modern SDKs. Cassettes are request/response pairs. Do we record the final assembled response, or the stream chunks? **Leaning:** record final response only for MVP. Stream replay is a Phase 7 problem (or never).

### 12.9 Secrets in cassettes

Cassettes contain full prompts and responses. Prompts often contain secrets-adjacent data (API keys pasted by users, file contents, etc.). Before `plexi replay export`, run a redaction pass. **Open:** what's the redaction rule? Probably: regex over known secret shapes + an opt-in app-declared redaction list in `replay.toml`.

### 12.10 Does stub mode need to be free?

Stub mode uses zero network. But the SDK still runs local Python / local code. Long pipelines in stub mode still take wall-clock time. For very large test matrices we may want a parallel executor. **Defer to Phase 4+** — MVP runs serially.

---

## Appendix A — Terminology

| Term | Meaning |
|---|---|
| Run | A single end-to-end execution of an app's agent pipeline. Has a `run_id`, a start/end time, and a directory on disk. |
| Span | An OpenLLMetry-compatible record of one LLM/image/tool call within a run. |
| Component | A named, iterable unit inside an app. Declared in `replay.toml`. Emits one or more spans per run. |
| Cassette | A serialized request/response pair used for replay. Keyed by content hash. |
| Fidelity mode | `stub` / `cheapest` / `default` / `pedal`. Declared per-run. |
| Suite | A named collection of test cases that runs at a declared minimum fidelity. |
| Fork | A new run derived from a parent run, re-executing from a chosen span onward with a declared swap. |
| Iterability | The property that changes to a component are observable under a given fidelity mode. |
| Meaningful | A test is meaningful for component C if the run's fidelity is high enough that C's output can affect the measured outcome. |

---

## Appendix B — Why we're not just using Langfuse / Inspect directly

Both are strong tools and we will likely use OpenLLMetry-compatible spans so they can consume our data. What they do not offer and we do need:

1. **The iterability gate.** No existing tool refuses to run a test because the test is meaningless at the chosen fidelity. This is the load-bearing idea.
2. **Cost-budgeted test selection** (Promptfoo has per-test cost assertions but not budget-driven selection across a suite).
3. **Fork-from-span-N with component swap** as a first-class operation (Braintrust and Langfuse have playgrounds, not full pipeline forks).
4. **Integration with Plexi's cost tracker, secrets, capabilities, and app sandbox.** An external tool can observe but cannot mediate.
5. **Replay UX as a Plexi app**, not a separate web UI the user has to context-switch to.

If the only problem was observability, we would use Langfuse. The load-bearing problems are selection and fidelity awareness, which are Plexi-specific because only Plexi owns the app manifest.

---

## References

- [LangSmith evaluation docs](https://docs.langchain.com/langsmith/evaluation)
- [Langfuse (OSS LLM engineering platform)](https://github.com/langfuse/langfuse)
- [Braintrust](https://www.braintrust.dev/)
- [Helicone caching](https://docs.helicone.ai/features/advanced-usage/caching)
- [Inspect (UK AISI)](https://inspect.aisi.org.uk/) / [inspect_ai on GitHub](https://github.com/UKGovernmentBEIS/inspect_ai)
- [Promptfoo](https://github.com/promptfoo/promptfoo)
- [OpenLLMetry / Traceloop semantic conventions](https://www.traceloop.com/docs/openllmetry/contributing/semantic-conventions)
- [VCR.py](https://vcrpy.readthedocs.io/) / [pytest-recording](https://github.com/kiwicom/pytest-recording)
- Plexi specs: [agent-orchestration.md](../subsystems/agent-orchestration.md), [app-infrastructure.md](../subsystems/app-infrastructure.md), [intelligence-protocol.md](../subsystems/intelligence-protocol.md)

# src/agent — Agent Contract

Read before editing anything under `src/agent/`: this file, `src/cli/AGENTS.md` (the CLI-facing
agent model: registry tiers, harness adapters, spawn path, provenance, `agent done`, escalation,
restore), and the root `AGENTS.md`.

This file defines the agent vocabulary — one agent, one noun. Every other doc references these
terms; it does not restate them.

## The agent is the definition; the harness is a property of it

- An **agent** is a *definition* — `AgentDefinition` (`src/agent/mod.rs`) — loaded from a directory
  containing `AGENT.md` (the persona / system prompt) and `settings.toml`, and discovered by
  `AgentRegistry`. `plexi agent create` writes one; `plexi agent show` reads one.
- A **harness** is the CLI the definition runs on: `AgentHarness` and its adapters in
  `HARNESS_ADAPTERS` (`src/cli/agent.rs`), carried on the definition as `AgentHarnessConfig`.
  `pi`, `claude`, `codex`, and `agy` are harness names, never agent names.
- The same definition on a different harness is the same agent. A harness name is never an identity,
  and an agent is not a kind of harness.

## Identity is not authority

They are two different artifacts doing two different jobs:

- **Identity / discovery — `AgentCard`** (`src/agent/mod.rs`), projected from a definition by
  `AgentCard::from_definition`. This is the local, A2A-shaped card `plexi agent list --json` prints.
  It says who an agent is. It grants nothing.
- **Authority — `AgentAuthorityRecord`** (a private `struct` in `src/agent/runs.rs`), persisted as
  `agent.json` and reached only through `write_definition` / `read_agent` /
  `list_agent_authorities`. It records an agent's declared tool grants. It is internal, not a public
  API, and it is never projected as a card.
- **A grant file is not a card.** `AgentCard` carries no grants; a card advertises, it does not
  authorize. Authority is a `GrantRecord` enforced by the permission monitor, and the mechanics —
  what a grant binds to, who decides, the runtime boundary — belong to
  [`docs/assistant-authority-model.md`](../../docs/assistant-authority-model.md). Do not restate them
  here.
- `AGENT.md` is guidance and grants nothing. A reporting line (`reports_to`) is presentation, not a
  grant.

## Lifetimes

There are three lifetimes, and only three:

- **ephemeral** — a definition plus a task, with no durable identity. Spawned by
  `plexi agent spawn --ephemeral` (`AgentHarnessConfig.ephemeral`), stamped `PLEXI_EPHEMERAL`, and
  auto-closed when it reports `agent done`. Its pane is gone; its return survives through
  `plexi agent result`.
- **named** — a definition with a durable id, resumable through its harness session
  (`SavedAgentSession`, `resume_command` on `pane info`), and visible to `plexi agent list` and the
  palette.
- **sub** — a *relation edge, not a kind*. A child is an ordinary agent whose run names a parent
  (`RunRecord.parent_run`, `parent_pane_id`); delegation and child runs in `src/agent/runs.rs` only
  narrow the parent's grants to a subset. There is no separate "sub-agent" kind.

## `default` is a definition, not a special case

`default` is an agent id like any other, and it obeys the same shadowing rule as every id: workspace
overrides global overrides built-in, resolved by `AgentRegistry::load_with_optional_workspace`, which
keeps the losing definition in `shadowed`. `ensure_workspace_default_agent` seeds a user-owned
`default` for a workspace on first use. So "the default agent for this directory" is not a different
kind of agent — it is simply the `default` definition the registry resolves there. To say which
definition a bare name resolves to, name the tier; never invent a distinct noun for it.

## "Team" is asked to mean three different things

Keep them apart; only the first is shipped:

- **Panel / consult — voters plus an aggregation rule.** This is the shipped meaning. `Team`
  (`src/agent/team.rs`) holds `options`, `Member`s, and `Ballot`s; `aggregate` and `report` produce
  the Borda result, and `plexi agent team run` runs it. It is deliberation only.
- **Run team — a roster plus a workflow** (a head, named members, declared budgets and policies).
  `proposed`, not shipped; the `Team` type has none of it.
- **Runtime spawn grouping** — grouping spawned panes at runtime. `proposed`, not shipped.

Do not use "team" in the present tense for a roster or a spawn group.

## Glossary

| Term | Means | Retired / do not use |
|---|---|---|
| agent | The definition (`AgentDefinition`) | a harness name as a synonym |
| harness | The CLI adapter a definition runs on (`AgentHarness`, `HARNESS_ADAPTERS`) | as a synonym for agent |
| card | The identity / discovery projection (`AgentCard`) | a grant file |
| authority | Declared grants (`AgentAuthorityRecord`), decided by the permission monitor | a card |
| ephemeral | Lifetime: definition + task, no durable identity | "unnamed agent", "ephemeral agent" |
| named | Lifetime: durable id, resumable, palette-visible | "named agent" where plain "agent" is meant |
| sub | Lifetime: a parent/child relation edge | "sub-agent", "subagent", "a kind of agent" |
| worker | A role: an agent dispatched to a task — not a kind or a lifetime | "worker agent" |
| default | An agent id subject to shadowing | "the default agent" as a special kind |
| team | A deliberation panel (shipped); a run roster or spawn grouping (`proposed`) | "team" for a spawn group in present tense |

## References

- `src/cli/AGENTS.md` — the CLI-facing agent model (registry tiers, harness adapters, spawn path,
  provenance, `agent done`, escalation, restore).
- [`docs/assistant-authority-model.md`](../../docs/assistant-authority-model.md) — authority
  mechanics: what the Assistant is trusted to do, who decides, what a grant binds to.
- `src/agent/team.rs` — the shipped deliberation `Team` (voters + aggregation rule).
- `src/agent/runs.rs` — runs, delegation, and the internal `AgentAuthorityRecord`.

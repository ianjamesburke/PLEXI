# Plexi Vision

> All the beauty of technology in one piece of software.

This document is the north star. Read it before making architectural decisions. It is not a roadmap — it is a compass.

---

## What Plexi Is

Plexi is a terminal multiplexer that hosts apps. One window. Everything else installs into it.

It is not a productivity suite. It is not a dashboard. It is the **environment itself** — a single surface where context, agents, permissions, and interfaces are all navigable without switching applications.

The goal is not feature completeness. The goal is a feeling: *I opened one thing and everything I need is here.*

---

## The Four Layers

```
INTERFACE         Plexi
                  One window. Apps install into ~/.plexi/apps/.
                  Context lives in ~/.plexi/backlog/ and .plexi/workspace.json.
                  The pane is the atom of attention.

ORCHESTRATION     Claude Code Skills + Agent Heads
                  Skills build, install, and operate Plexi apps.
                  Agent heads are directory watchers that auto-execute pipelines
                  when tasks land in their queue. Same trust network model as
                  the video production SDK — tasks propagate up the hierarchy.

EXECUTION         Sub-agents
                  Haiku → cheap scanning, triage, backlog synthesis
                  Sonnet → building, writing, wiring
                  Opus → hard decisions, architecture
                  Always disposable. Always return results to Plexi.

MEMORY            DEV_LOG.md + ~/.plexi/backlog/ + .plexi/workspace.json
                  DEV_LOG = per-project decision journal (Claude Code sessions)
                  backlog = raw captures before they become tasks
                  workspace = spatial layout + app state, scoped to directory
```

---

## The Three Problems Being Solved

### 1. The Context Problem

Switching projects means re-establishing context: which branch, what was last touched, what decisions were made, what's blocked. The solved version is a Plexi where that context is always present — surfaced by apps that know the project, agents that read the logs, and a backlog that feeds into both. Directory-scoped workspace persistence (`.plexi/workspace.json`) is the foundation. Everything else builds on it.

### 2. The Permission Problem

Agents calling agents calling tools calling APIs — the chain of trust must be visible and auditable. Plexi's capability manifest (`manifest.toml`) is the foundation: `filesystem`, `terminal_write`, `network`, `secrets` — all explicit, all minimal by default. The solved version is a trust network where each agent head has a declared capability scope, elevating permissions is a deliberate act, and you can always see exactly what any app or agent can do.

### 3. The Interface Problem

The terminal is already the right place to work. Plexi makes it the only place you need — not by replacing everything, but by being the hub. Browser, notes, agents, files, audio, notifications — accessible without switching windows. The pane model is the key primitive: panes track context, panes receive notifications, panes are the unit of focus.

---

## The Agent Pipeline Vision

The long-form version of the orchestration layer:

1. A quick note lands in `~/.plexi/backlog/`
2. The Backlog Triage app (a Plexi app) surfaces it — keyboard navigation, route with a single keypress
3. Routing sends the note to an **agent head directory** (e.g. `~/.plexi/agents/plexi-dev/`)
4. The agent head picks up the task automatically and starts its pipeline
5. Progress and follow-up questions propagate up through the **unified notification stream**
6. The notification stream is a single pane — all active agent threads, grouped by context proximity
7. The user stays focused on one thing; background work surfaces only when it needs attention

This is the same trust network as the video production SDK — tasks have declared scope, agents have declared capabilities, the hierarchy is explicit.

---

## The Focus Vision

Plexi tracks where attention is spent because it owns the pane model. This data can teach you things:

- How many contexts can you actually juggle before decision quality drops?
- How long does a particular type of decision take you?
- Which pane transitions correlate with productive output vs. thrashing?

The solved version is a lightweight background app that reads pane focus events, infers semantic context from pane content (CWD, app type, open files), and surfaces patterns — not as interruptions, but as periodic insight. "You've been switching between 4 contexts for 2 hours — usually when this happens you're blocked on X."

Flow-state optimization follows from this: when agents are churning in the background, Plexi knows. It can surface work that's in a different context area (non-overlapping file scope) so you're never blocked, always moving.

---

## The Predictive Layer Vision

The most speculative piece. Borrowing from the video production aiden infrastructure:

- Plexi observes your decision patterns across sessions
- Before you act, it makes a prediction — what are you probably about to do?
- It judges its prediction against what you actually do
- Over time, it gets better at pre-loading context, pre-running agents, and surfacing the right thing before you ask

This is not a chatbot. It is ambient intelligence — invisible when it's working, visible only when it's wrong or when it wants to surface something.

---

## Principles

**One home for everything.** Every tool, agent call, permission, and context has a place in Plexi. If something doesn't have a home, the answer is usually: build an app.

**Apps are the unit of distribution.** A Plexi app is a directory with a `manifest.toml` and a script. That's the atom. Skills build them, agents run them, users install them by dropping a folder.

**Panes are the unit of attention.** Focus tracks through panes. Context is scoped to panes. Notifications arrive in panes. The pane model is load-bearing — architectural decisions should preserve it.

**Agents are ephemeral, architecture is permanent.** Sub-agents are cheap and disposable. The manifest, the skill, the DEV_LOG entry — those persist. Optimize for the artifact, not the conversation.

**Permissions feel like trust, not friction.** Minimal by default. Explicit when elevated. Never paper over missing permissions — fail fast, surface what's needed.

**Beautiful is not cosmetic.** The CRT scanlines, the ghost opacity, the accent color are signals that Plexi cares about the experience of using it. New apps should feel like they belong. If something feels clunky, it needs a better app, not a workaround.

**Optimize for flow, not throughput.** The right metric is not "how many things got done" but "how few times did the user get pulled out of deep focus." Background agents, smart notification grouping, and context-aware work surfacing all serve this.

---

## What Does Not Belong Here

- Features that exist to be features
- Apps that duplicate what the terminal already does well
- Any system that requires maintaining two sources of truth
- Complexity that isn't in service of one of the three problems above

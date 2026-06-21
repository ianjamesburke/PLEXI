# Plexi North Star

> **Plexi — the last app you'll ever need.**

This is the compass for Plexi's development. It is not a sprint plan or a feature list. It is the guiding vision — what Plexi is trying to be, why it exists, and the direction everything should move toward. Read this before filing issues, implementing features, or making architectural decisions.

---

## The One-Liner

**The last app you'll ever need.** Not because it does everything today. Because you never outgrow it. You grow it.

---

## Short Bio

Plexi is a tiling, terminal-native personal computing environment. You run capability-gated apps inside it, build apps for it, and wire them together with typed pipes and shared context. Everything lives on disk. AI, audio, network, and filesystem access are brokered with explicit consent and a full audit trail. You install it once and build your working environment on top of it for the rest of your life.

---

## Long Bio

The premise of Plexi is that modern software has the ownership relationship backwards. You subscribe to tools you can never modify, that store your data somewhere you can't inspect, and that disappear the moment you stop paying. Plexi is the opposite bet: a computing environment you own completely, the way you own a book.

Software was built for human interaction first. Every interface, every workflow, every paradigm was designed around a person clicking, typing, and waiting. When AI arrived, it got duct-taped to the side — a chat window bolted onto an app that was never built to reason about itself, modify its own behavior, or hand work back and forth with an agent. The result is that neither participant gets what they need. The human is still fighting the interface; the AI is operating blind.

The whole paradigm needs to be re-imagined. Not for AI alone — for collaboration. Getting the most out of a human and the most out of an AI requires software that can act as a garden: a living, modular, breathing protocol where both participants can tend the environment, hand off control naturally, and grow interfaces that fit the individual using them. Not a universal UI. A personal one, shaped by use.

Plexi is built for that model from the ground up. At the core is a tiling layout with app panes and terminal panes. The host Assistant is a first-party builtin: a workspace operator with direct access to the pane tree, filesystem, terminal panes, app connectors, and the unified permission broker. It is the place where a user asks Plexi to do work, and it should feel like Claude Code but for your entire computing environment, not just a git repo. Third-party AI apps are normal PGAP apps that use `ai.query` as a capability; they do not get ambient workspace control. Apps do not share memory or state with the host. PGAP/Python apps speak the native JSON protocol as reviewed local processes. WASM apps speak the typed component-model runtime with link-time host imports. Both use explicit capability grants, scoped decisions, and audit logs. Python apps remain native subprocesses unless they move to the WASM compatibility path.

The app contract is the key abstraction. PGAP is the native protocol: any process that speaks newline-delimited JSON can be a Plexi app. WASM is the sandbox/performance runtime: any component that implements the WIT lifecycle can be a Plexi app. This means apps are portable, auditable, and replaceable, and agents can invoke them, wire them together, and build new ones without leaving the environment.

Everything persists on disk in formats that will still open in 100 years. Your app state, your permissions log, your agent transcripts — all plain files on your machine. If Plexi disappeared tomorrow, your data and your apps would still be there. That is not a feature. That is the founding constraint.

The long horizon: Plexi becomes a platform. You build apps, publish them, sell them. Others install what you made. The environment is inheritable — hand someone your laptop and you are handing them your entire working setup, shaped exactly to how you think.

---

## Who This Is For

Plexi's primary audience is non-technical people aware of the potential of AI but who know they could be getting more out of it. They may not know exactly what the terminal does or what it's capable of, but they know it's what AI power users have started to catch on to.

Plexi meets them where they are. 99% of CLI commands are executed by agents, not humans. The entire user experience is wrapped in a command-line interface so that an agent can use it, collaborate with you to build any interface you can imagine, and run it on any computer for the next hundred years. Every feature that a power user would wire up manually — script discovery, secret injection, command listing, argument forwarding — Plexi packages as a single entry with guardrails, discoverability, and a growth path. `plexi run list` teaches a new user what's available. `plexi run edit` gives them a door into customization without requiring them to understand shell configuration. The CLI is not a power-user shortcut — it's the primary interface for agents, and it must be learnable by someone whose first terminal was last month.

The secondary audience is **power users and agent builders** who need a programmable, auditable, agent-scriptable environment. They benefit from the same product abstractions — a command registry that agents can read, manifests that declare capabilities, structured metadata that tooling can index — even though they could wire the pieces themselves.

The primitives are right when you can build a deployable agentic system and ship it directly onto your client's hardware. No authentication layer to maintain. No cloud costs to absorb. No infrastructure to babysit. If the system needs AI, you hook them up with an OpenRouter account of their own and they're set up, good to go. The deployment model is: install Plexi, install the app, connect the API key. Done. You built the system; they own it on their machine.





**Design principle — product over primitive:** when a Plexi feature overlaps with a UNIX primitive (PATH, make, env), that overlap validates the abstraction — it means the concept is sound. The product layer on top (discoverability, scope labeling, secret injection, agent indexing) is the value. Never decline to build a feature because a power user could assemble it from parts. The target user can't, and the power user benefits from the structured version anyway.

**Design principle — the CLI is the product:** the CLI is not a developer escape hatch. It is the primary interface for both humans and agents. Every command must be self-documenting: help text written for someone who started coding last month, not someone who's read the POSIX spec. No jargon without explanation. No dead commands. No stubs. Every `--help` is a teaching moment — it explains not just what the command does but *why you'd use it* and *what it connects to*. Agents read the same help text to discover capabilities, so completeness and accuracy are load-bearing. A CLI tips system (like git's hints) can guide new users toward the next thing to try — togglable in config.toml for power users who don't need it.

---

## The Progression

### Phase 0 — Foundation *(shipped)*

Tiling layout. Terminal panes, app panes. PGAP protocol v3 + Python SDK. Typed pipes. Context/workspace scoping. Capability system with permissions.jsonl audit trail. Copy-mode. CLI. File browser. QuickNote. Text editor pane.

### Phase 1 — Stabilize & Polish *(in progress)*

The foundation works. Now make it solid. Declarative keybinding table (eliminate the subset-match footgun). Unified modal system (kill copy-paste scrim patterns). CLI namespace finalization (`pane new`, `app open`). Focus system unification. Welcome screen redesign. Text editor upgrade (extract QuickNote's battle-tested input handling into a shared primitive). Notification polish. Core app theming consistency.

### Phase 2 — The Protocol *(next)*

PGAP becomes the product's defining abstraction. **L1-only declarative UI**: apps send a tree of semantic nodes (Stack, List, AppBar, Footer, Button, Input, etc.); the host handles all layout, spacing, theming, scrolling, focus, and hit testing. No pixel math in apps. The `Raw` node survives as the escape hatch for custom rendering (visualizations, games). L0 flat draw commands and `_l0` fallback fields are removed.

The goal: a non-technical person sits down with an AI assistant, says "give me a Plexi app that does X," and it works on the first try because the SDK is a tree builder with 10 concepts, not a rendering engine with 40. A functional, visually polished app in under 100 lines of Python.

**Security model**: the capability system (consent + permissions.jsonl audit trail) is the v1 enforcement layer. Python process sandboxing is not attempted at the language level; the protocol brokers host APIs, and the marketplace review process catches abuse. v1 marketplace apps are reviewed native processes. True process isolation comes with the v2 WASM runtime. This is an acknowledged gap, not an oversight.

The SDK's job after this phase: state management, tree building, event dispatch. That's it.

### Phase 3 — Intelligence *(near term)*

The host Assistant becomes a full workspace operator. It reads and writes files, searches code, sends commands to terminal panes, calls app connectors, and manages panes, all through typed host tools gated by the unified permission broker. Permission postures (plan, review, work, locked) set the baseline once; the broker auto-evaluates every tool call against the active posture, agent-specific grants, and persisted user decisions. In work mode, common file and terminal operations flow without interruption, matching the velocity of dedicated coding assistants.

Agents are named personas inside the host Assistant: different system prompts, model tier policies, tool sets, permission postures, and optional skills. A builder agent has file tools and terminal access in work mode. A spreadsheet agent has app connector tools for CSV operations and a constrained permission set. Agents are selectable, not installable apps. The agent registry lives on disk (user-level and workspace-level), and marketplace agent packages install into the same registry.

Third-party AI apps remain normal PGAP apps. They use `ai.query` as a capability, build their own UI, and ship to the marketplace like any other app. They do not get ambient workspace control. The host Assistant is the privileged operator; third-party agents are apps.

Skills are markdown-backed procedures the assistant can invoke manually (`/skill-name`) or auto-select by description match. App connectors expose typed read/write tools from running apps (e.g. `csv.read_range`, `csv.write_range`). Tool collections group related tools for install and enablement. These three mechanisms (agents, skills, connectors) compose: a builder agent enables coding skills and terminal tools; a data agent enables spreadsheet connectors and analysis skills.

AI onboarding for non-technical users: hardware scanning, local model recommendation and setup (Ollama), guided API key entry for cloud providers, and eventually a Plexi-managed subscription backend so `ai.query` works out of the box with zero configuration.

Ephemeral pane manager: panes that are alive but not in the active tiling layout. They run in memory, processes stay alive, notifications still fire. Cmd+I summons the inventory overlay (searchable, keyboard-navigable). Pull a pane back into your layout or let it run in the background. The "backpack" that comes with you across contexts.

### Phase 4 — The Platform *(medium term)*

WASM app runtime with WASI capability mapping. Same app contract, different runtime: typed component-model calls instead of PGAP JSON over stdio. True process sandboxing via WASM. `Surface { id }` node for direct GPU rendering (games, real-time visualizations). This is the performance tier: apps that need 60fps with hundreds of objects target WASM; apps that need simplicity target PGAP/Python. Both ship to the same marketplace.

App marketplace: `plexi app dev` (hot-reload local development), `plexi app publish` (package + upload), `plexi app install <name>` from registry. Submission review flow. Revenue sharing for app authors. v1 starts with reviewed native-process apps and blunt trust labels. v2 adds WASM apps under the same install flow once the sandbox is real.

### Phase 5 — The Portable Instance *(long term)*

A Plexi instance runs as a server: on your local machine, on a cloud VM, on rented GPU hardware. A thin client connects from any Mac, Windows, or Linux machine and renders the UI. Detach from one machine, attach from another. Your config, apps, dotfiles, secrets index, agent transcripts all live on the server instance.

SpacetimeDB as the persistence and sync layer. Not just syncing config across machines, but hosting the entire runtime. Rent a GPU box for video rendering, run agents on beefy hardware, then jump to your laptop at a coffee shop. Self-hosting and cloud-hosting use the same architecture; the only difference is where the server runs.

The deployment model: your Plexi environment is a portable server. Cloud is an option, never a requirement. Local-first is preserved because you can always run the server on your own hardware.

### Phase 6 — Ubiquity *(horizon)*

Linux (it's a Rust binary; it should already work). Windows. Non-traditional form factors. The same configuration, the same keybindings, the same apps, on any device that can run a process and render a frame.

### Phase ∞ — The Inheritance Layer

A Plexi environment is a complete record of how a person works. Every app, every permission, every agent transcript, every piece of state, on disk, portable, ownable. You hand someone your machine, you hand them your working life. You hand the next generation your accumulated workflows the way you hand them a library.

That has never been true of software before.

---

## What Does Not Belong Here

- Cloud state or cloud-required features *(cloud sync is opt-in and portable; local-first is never compromised)*
- Apps that duplicate what a terminal already does well
- Capabilities that require trusting the app rather than the protocol *(the v1 Python SDK relies on consent + audit, not process isolation; true sandboxing arrives with the WASM runtime; the host Assistant is the sole exception: it is a first-party builtin with direct host access, not a protocol app)*
- Any system that creates two sources of truth for state or permissions
- Complexity that serves the implementation rather than the capability
- Universal UIs designed for everyone — Plexi grows to fit the individual
- Pixel math in app code — the host handles layout, spacing, theming, and rendering; apps declare structure

---

## When to Read This

- When starting a new Plexi work session
- Before building a new app — does it fit the local-first, own-it-completely model?
- Before adding a protocol primitive — does it serve both human and machine callers?
- Before filing or triaging an issue — does this move toward the garden, or away from it?
- When an architectural decision feels wrong — come back to "local-first is non-negotiable"
- For the app-framework and marketplace execution plan, see [`docs/app-framework-marketplace.md`](docs/app-framework-marketplace.md).

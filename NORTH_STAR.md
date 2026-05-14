# Plexi North Star

> **Plexi — the last app you'll ever need.**

This is the compass for Plexi's development. It is not a sprint plan or a feature list. It is the guiding vision — what Plexi is trying to be, why it exists, and the direction everything should move toward. Read this before filing issues, implementing features, or making architectural decisions.

---

## The One-Liner

**The last app you'll ever need.** Not because it does everything today. Because you never outgrow it. You grow it.

---

## Short Bio

Plexi is a tiling, terminal-native personal computing environment for macOS. You run sandboxed apps inside it, build apps for it, and wire them together with typed pipes and shared context. Everything lives on disk in plain text — no cloud, no subscriptions, no data held hostage. AI, audio, network, and filesystem access are brokered with explicit consent and a full audit trail. You install it once and build your working environment on top of it for the rest of your career.

---

## Long Bio

The premise of Plexi is that modern software has the ownership relationship backwards. You subscribe to tools you can never modify, that store your data somewhere you can't inspect, and that disappear the moment you stop paying. Plexi is the opposite bet: a computing environment you own completely, the way you own a book.

Software was built for human interaction first. Every interface, every workflow, every paradigm was designed around a person clicking, typing, and waiting. When AI arrived, it got duct-taped to the side — a chat window bolted onto an app that was never built to reason about itself, modify its own behavior, or hand work back and forth with an agent. The result is that neither participant gets what they need. The human is still fighting the interface; the AI is operating blind.

The whole paradigm needs to be re-imagined. Not for AI alone — for collaboration. Getting the most out of a human and the most out of an AI requires software that can act as a garden: a living, modular, breathing protocol where both participants can tend the environment, hand off control naturally, and grow interfaces that fit the individual using them. Not a universal UI. A personal one, shaped by use.

Plexi is built for that model from the ground up. At the core is a tiling layout with three pane types — terminal (PTY), app (PGAP process), and agent (LLM loop). Apps don't share memory or state with the host; they communicate over a typed protocol with explicit capability grants. A sandboxed app can render UI, play audio, open a browser, query an LLM, read your filesystem — but only if you said yes, and only in the scope you granted. Every decision is logged.

The protocol — PGAP, Plexi Generic App Protocol — is the key abstraction. Any process that speaks newline-delimited JSON can be a Plexi app. The Python SDK wraps it, but the protocol is the primitive. This means apps are portable, auditable, and replaceable — and agents can invoke them, wire them together, and build new ones without leaving the environment.

Everything persists on disk in formats that will still open in 100 years. Your app state, your permissions log, your agent transcripts — all plain files on your machine. If Plexi disappeared tomorrow, your data and your apps would still be there. That is not a feature. That is the founding constraint.

The long horizon: Plexi becomes a platform. You build apps, publish them, sell them. Others install what you made. The environment is inheritable — hand someone your laptop and you are handing them your entire working setup, shaped exactly to how you think.

---

## Who This Is For

Plexi's primary audience is **vibe coders** — people who build with AI, ship fast, and are just discovering what the terminal can do. They aren't sysadmins. They don't have a `.zshrc` they've cultivated for a decade. They may not know what PATH is. But they know they want to build things, and they showed up because an AI helped them write their first script.

Plexi meets them where they are. Every feature that a power user would wire up manually — script discovery, secret injection, command listing, argument forwarding — Plexi packages as a product experience with guardrails, discoverability, and a growth path. `plexi run list` teaches a new user what's available. `plexi run edit` gives them a door into customization without requiring them to understand shell configuration. The CLI is not a power-user shortcut — it's the primary interface, and it must be learnable by someone whose first terminal was last month.

The secondary audience is **power users and agent builders** who need a programmable, auditable, agent-scriptable environment. They benefit from the same product abstractions — a command registry that agents can read, manifests that declare capabilities, structured metadata that tooling can index — even though they could wire the pieces themselves.

**Design principle — product over primitive:** when a Plexi feature overlaps with a UNIX primitive (PATH, make, env), that overlap validates the abstraction — it means the concept is sound. The product layer on top (discoverability, scope labeling, secret injection, agent indexing) is the value. Never decline to build a feature because a power user could assemble it from parts. The target user can't, and the power user benefits from the structured version anyway.

**Design principle — the CLI is the product:** the CLI is not a developer escape hatch. It is the primary interface for both humans and agents. Every command must be self-documenting: help text written for someone who started coding last month, not someone who's read the POSIX spec. No jargon without explanation. No dead commands. No stubs. Every `--help` is a teaching moment — it explains not just what the command does but *why you'd use it* and *what it connects to*. Agents read the same help text to discover capabilities, so completeness and accuracy are load-bearing. A CLI tips system (like git's hints) can guide new users toward the next thing to try — togglable in config.toml for power users who don't need it.

---

## The Progression

### Phase 0 — Foundation *(shipped)*

Tiling layout. Terminal, App, and Agent pane types. PGAP protocol + Python SDK. Typed pipes. Context/workspace scoping. Capability system with permissions.jsonl audit trail. Copy-mode. CLI.

### Phase 1 — Intelligence & Wiring *(in progress)*

IQ query: agent panes with OpenRouter backend, ledger per turn, session transcripts on disk. Ollama backend (local LLM, no OpenRouter dependency). Typed pipes Phase 1: manifest `[app.io]` wiring, auto-wire by linked group.

### Phase 2 — Agent Infrastructure *(near term)*

`[app.skill]` manifest — apps declare how agents invoke them. `[app.agent]` manifest — installable agent apps with system prompt + tool allowlist. Agent-invokable app registry. Trust tiers: peer, subordinate, orchestrator. Agents that can spawn panes, wire pipes, and hand off to other agents.

### Phase 3 — The Platform *(medium term)*

WASM app support with WASI capability mapping — portable apps that run anywhere. App store / marketplace: install, publish, sell. Revenue sharing for app authors.

### Phase 4 — The Portable Identity *(long term)*

Your Plexi environment is yours everywhere. SpacetimeDB as the sync layer — not just for collaboration but for identity portability. Every keybinding, every app, every permission, every configured workflow syncs across machines. Open any machine running Plexi, pull in your interface, and be at home in seconds.

A tarball builder for your complete environment: zsh config, dotfiles, shell history, terminal setup — packaged and versionable alongside your apps. Cloud-hosted configuration with a security model that matters: you can offload your config onto any local Plexi instance without bridging it to your core infrastructure. Workflows travel; your secrets don't have to.

This is the higher-order OS layer. Not an app running on macOS. A meta-environment that sits above whatever operating system is underneath — so that high-impact individuals get the most out of their time in front of any computer, because their environment came with them.

### Phase 5 — Ubiquity *(horizon)*

Linux (it's a Rust binary; it should already work). Windows. Wearables and non-traditional form factors. The same configuration, the same keybindings, the same apps — on any device that can run a process and render a frame.

### Phase ∞ — The Inheritance Layer

A Plexi environment is a complete record of how a person works. Every app, every permission, every agent transcript, every piece of state — on disk, portable, ownable. You hand someone your machine, you hand them your working life. You hand the next generation your accumulated workflows the way you hand them a library.

That has never been true of software before.

---

## What Does Not Belong Here

- Cloud state or cloud-required features *(cloud sync is opt-in and portable; local-first is never compromised)*
- Apps that duplicate what a terminal already does well
- Capabilities that require trusting the app rather than the protocol
- Any system that creates two sources of truth for state or permissions
- Complexity that serves the implementation rather than the capability
- Universal UIs designed for everyone — Plexi grows to fit the individual

---

## When to Read This

- When starting a new Plexi work session
- Before building a new app — does it fit the local-first, own-it-completely model?
- Before adding a protocol primitive — does it serve both human and machine callers?
- Before filing or triaging an issue — does this move toward the garden, or away from it?
- When an architectural decision feels wrong — come back to "local-first is non-negotiable"

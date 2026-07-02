# Plexi North Star

> **Plexi — the last app you'll ever need.**

This is the compass, not a plan. Read it before filing issues, implementing features, or making architectural decisions.

---

## The One-Liner

**The last app you'll ever need.** Not because it does everything today. Because you never outgrow it. You grow it.

Plexi is a mini-OS, not an app: a tiling, terminal-native personal computing environment where capability-gated apps, agents, and terminals share one inheritable place. You install it once and build your working environment on top of it for the rest of your life. Software has the ownership relationship backwards — you subscribe to tools you can't modify, storing data you can't inspect, that vanish when you stop paying. Plexi is the opposite bet: an environment you own completely, the way you own a book. And it is built for human + AI collaboration from the ground up — a garden both participants tend, growing interfaces that fit the person using them — not a chat window bolted onto software that was never built to reason about itself.

---

## The Ten Commandments

1. **All data lives in portable, open formats — markdown, JSON, TOML.** App state, permission logs, agent transcripts: plain files on your disk that will still open in 100 years. If Plexi disappeared tomorrow, everything you made would still work. That is the founding constraint, not a feature.

2. **Ergonomics are a priority, not an afterthought.** Friction is a bug with the same severity as a crash. Layout, focus, chrome, and defaults are designed deliberately — polish is not something that happens after the feature ships.

3. **Keyboard-first. The mouse is optional.** Every interaction has a keyboard path, and the fast path is the default path.

4. **Every Plexi feature is reachable through the Plexi CLI.** 99% of commands are run by agents. The CLI is the product — the primary interface for humans and agents alike, never a developer escape hatch. If a feature isn't in the CLI, it doesn't exist for the agent.

5. **`--help` covers the entire surface.** An agent — or a person whose first terminal was last month — can learn all of Plexi without leaving it. Help text explains what, why, and what it connects to. It is load-bearing: agents discover capabilities by reading it.

6. **If it needs a manual, it needs a redesign.** No jargon without explanation, no dead commands, no stubs. Obvious over clever.

7. **Your environment is an inheritance.** A Plexi instance is a complete record of how a person works — every app, permission, transcript, and piece of state, portable and ownable. Hand someone your machine, and you hand them your working life. Hand the next generation your workflows the way you'd hand them a library. That has never been true of software before.

8. **The whole app lifecycle lives in Plexi — build, test, publish, iterate.** An agent scaffolds an app, hot-reloads it, tests it, and ships it to the marketplace without leaving the environment. Apps are portable, auditable, replaceable; any process that speaks the protocol is an app.

9. **Local-first. Cloud is an option, never a dependency.** Sync, hosting, and the portable server instance are opt-in conveniences on the same architecture. Nothing required to run what you own lives on someone else's computer.

10. **Apps never get ambient authority.** Every capability is declared in a manifest, granted with explicit scoped consent, and logged to an append-only audit trail. The host owns rendering and enforcement; apps own state and intent. Security by construction, not by policy.

---

## Design Principles

**Product over primitive:** when a Plexi feature overlaps a UNIX primitive (PATH, make, env), the overlap validates the abstraction. The product layer — discoverability, scope labeling, secret injection, agent indexing — is the value. Never decline to build a feature because a power user could assemble it from parts; the target user can't, and the power user benefits from the structured version anyway.

**Grown, not universal:** no universal UI. Plexi grows to fit the individual, shaped by use, with the assistant as workspace operator and third-party AI apps as ordinary capability-gated apps — never ambient controllers.

---

## Who This Is For

Primary: non-technical people who know AI power users have caught on to the terminal and want that leverage without the priesthood. Plexi wraps the whole experience in a CLI an agent can drive and a person can learn. Secondary: power users and agent builders who need a programmable, auditable environment — and a deployment model of "install Plexi, install the app, connect a key. Done."

---

## The Progression

- **Foundation** *(shipped)* — tiling layout, terminal + app panes, PGAP v3 + Python SDK, typed pipes, contexts, capability system with audit trail, CLI, core apps.
- **Phase 2 — The Protocol** *(now)* — declarative host-rendered UI; a polished app in under 100 lines of Python; v1 security = consent + audit + review (true sandboxing arrives with WASM — acknowledged gap, not oversight).
- **Phase 3 — Intelligence** — the host Assistant becomes a full workspace operator: typed host tools behind the permission broker, named agent personas, skills, app connectors, AI onboarding for non-technical users.
- **Phase 4 — The Platform** — WASM runtime (same app contract, true sandboxing, mobile-viable), marketplace with review flow and revenue sharing.
- **Phase 5 — The Portable Instance** — your environment runs as a server, local or cloud, same architecture; thin clients attach from anywhere; SpacetimeDB persistence/sync.
- **Phase 6 — Ubiquity** — Linux, Windows, any device that can run a process and render a frame. Same config, same keybindings, for life.
- **Phase ∞ — The Inheritance Layer** — commandment 7, fully realized.

---

## What Does Not Belong Here

- Cloud-required features (sync is opt-in and portable)
- Apps that duplicate what a terminal already does well
- Capabilities that require trusting the app rather than the protocol
- Two sources of truth for state or permissions
- Complexity that serves the implementation rather than the capability
- Pixel math in app code — apps declare structure; the host renders

For the app-framework and marketplace execution plan, see [`docs/app-framework-marketplace.md`](docs/app-framework-marketplace.md).

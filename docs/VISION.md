# Plexi Vision

> All the beauty of technology in one piece of software.

## The Foundational Claim

Every piece of software ever built was designed for humans to operate directly. AI has been grafted onto that assumption — chat boxes appended to CRMs, "AI features" bolted onto word processors, recommendation engines bolted onto search. This is e-commerce duct-taped to a physical storefront.

The internet didn't improve retail. It created Amazon. Businesses that won the internet age weren't businesses that added e-commerce to their existing model — they were internet-native businesses that completely reimagined what business could be on the new substrate. The businesses that lost were the ones that duct-taped a website to their physical operation.

We are in that moment now, with agents.

The right move is not to add AI to software designed for humans. It is to reimagine what software is when agents are first-class users of it — and build that from scratch.

**Plexi is that reimagination.**

## What This Means

Plexi is the first agent-native computing environment. A terminal multiplexer in form. A new substrate in function.

An app in Plexi is not a UI. It is a **capability** — something that can be used by a human, an agent, or both simultaneously, through the same install, with the same permissions, on the same protocol.

The human is not removed from the loop. They are elevated above it.

You see what you need to see. You say what you want. Plexi does it. When it's ambiguous, it asks. When it needs permission, it shows you exactly what it's about to do and waits for you.

## The Non-Negotiables

**Agent-native first, human-friendly always.**
Every capability must be invokable by an agent. The UI is the human face of a capability, not the capability itself.

**One install, three interfaces.**
A single app directory is a UI, a skill, and potentially an agent. Nothing is duplicated.

**The permission model is the product.**
Every capability declared, every permission granted, every LLM call logged. Not as friction — as legibility.

**PGAP is the only path to intelligence.**
No app calls Anthropic directly. No agent makes uncounted LLM calls. Everything routes through PGAP.

**Beautiful is not cosmetic.**
The draw protocol enforces discipline. If something looks bad, fixing it means improving the capability.

**Directory is the permission boundary.**
When you launch Plexi inside a directory, everything installed, spawned, or agentic within that scope is provably incapable of reaching anything outside it. Parent directories, sibling projects, and the wider filesystem are invisible by construction — not by convention, not by policy, not by "we promise." A subdirectory Plexi is a sealed box, and the seal is the product.

---

*This file is the source of truth for Plexi's foundational vision. Do not paraphrase it in other documents — reference it directly. The north star compass lives at `~/.agents/skills/plexi-north-star/SKILL.md` and builds on this foundation.*

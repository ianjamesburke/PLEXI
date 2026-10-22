---
title: "Agents still need a cockpit — and it can be your terminal"
date: 2026-10-21
description: "A response to JetBrains on IDEs and agents: keep verification, context, and human judgment close to the work in a terminal-first workspace."
cover: ../../../../public/og-default.png
draft: true
---

Coding agents need a workspace where you can see what happened and decide what happens next. I agree with JetBrains on that.

In [Why We Still Need an IDE](https://blog.jetbrains.com/blog/2026/10/20/why-we-still-need-an-ide/), Isaac Levin writes: “The engineer owns what ships, even when the agent wrote most of the code.”

Yes. A plausible summary from an agent is a starting point for review. You still need to inspect the change and check the result.

Levin argues that an IDE brings code navigation, analysis, and verification into a connected view. He also makes the case for choosing your agents and bringing your own provider credentials. Those are useful requirements for the workspace around an agent.

I think a terminal can be the starting point for that workspace too.

## Keep the work in view

I already run agents, compilers, tests, and Git from a shell. I want to keep those tools close together, with room for notes and interfaces that help me understand the result.

That is why I am building [Plexi](/). It puts terminal panes and [local apps](/docs/apps) in the same workspace. Its [CLI](/docs/cli) lets tools work with the host, and agent status hooks give them a way to report progress. Terminal-based agents keep their own accounts or API keys.

Plexi does not yet offer the integrated code intelligence Levin describes. If I need an IDE's debugger or navigation, I can open one. The terminal is where I want to organize the work around those tools.

The test for this agent cockpit is practical: can I see the change, run the relevant check, and make an informed decision? A green test still leaves me responsible for whether we built the right thing.

That responsibility stays with me wherever I put the window.

[Try Plexi](/download) and follow the [getting-started guide](/docs/getting-started).

---
name: agent-handoff-summary
description: "Summarize active agent panes from host-managed pane slots: what each lane is doing, waiting on, and needs from Ian."
risk: low
source: local
date_added: "2026-06-11"
---

# Agent Handoff Summary

Use when Ian asks what agents are doing, what is waiting, or what needs his attention.

Run:

```bash
.agents/skills/_lib/agent-handoff-summary.sh
```

The helper reads `plexi pane list` slot metadata and prints one entry per pane that has pipeline slots.

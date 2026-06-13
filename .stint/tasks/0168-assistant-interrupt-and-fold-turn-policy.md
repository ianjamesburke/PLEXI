---
id: "0168"
title: "Assistant interrupt-and-fold turn policy + act-on-your-turn prompt"
status: todo
estimate: "4h"
sprint: "s5"
blocked_by: []
gh_issue:
  - "2204"
area:
  - "host/ai"
tags:
  - "v1"
---


## What

Add a cancellation seam to the AI broker turn loop and use it in the host
Assistant: a user event or message arriving mid-turn cancels the in-flight
streaming turn and folds queued context into one immediate follow-up turn
(replacing queue-behind, which produced 6-26s lag in chess testing). Extend
ASSISTANT_SYSTEM_PROMPT so the assistant acts (calls the tool) when a
delivered event hands it the turn, instead of only commenting.

## Why

Live responsiveness to app events is the flagship behavior of the agent
platform; queue-behind lag and comment-instead-of-act failures make the demo
feel broken even though event plumbing is correct.

## References

- GitHub issue #2204
- src/assistant/mod.rs (start_turn, pump_turn_io follow-up dispatch)
- src/plexi_ai/broker.rs, src/plexi_ai/loop.rs (cancel token seam)

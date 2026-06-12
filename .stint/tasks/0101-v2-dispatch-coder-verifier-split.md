---
id: "0101"
title: "v2 dispatch: coder/verifier split"
status: backlog
sprint: "s23"
estimate: 6h
blocked_by:
  - 147
gh_issue: ["1446"]
area: ["infra/skills", "infra/agents"]
tags: ["v2", "dispatch", "agents"]
---

Split implementation and headless verification into separate agent roles so dispatch can use model-tiering and independent review.

Sequenced after `0147` because non-blocking validation notifications should land before deeper dispatch role changes.

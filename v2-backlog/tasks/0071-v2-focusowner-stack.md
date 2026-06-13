---
id: "0071"
title: "v2 host architecture: unified FocusOwner stack"
status: backlog
sprint: "s15"
estimate: 8h
blocked_by:
gh_issue: ["1238"]
area: ["host/pane-ops"]
tags: ["v2", "input", "architecture"]
---

Collapse host `FocusLayer` and app keyboard capture into one explicit focus-owner stack.

## Why

This is the prerequisite for a real input router and for terminal/input fixes that cannot be solved while multiple consumers pull from the same egui event queue.

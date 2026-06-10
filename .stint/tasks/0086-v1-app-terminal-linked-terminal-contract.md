---
id: "0086"
title: "v1 app-terminal: linked terminal contract"
status: backlog
sprint: "s3"
estimate: 10h
blocked_by: ["0014"]
blocked_by_gh: []
gh_issue: ["599"]
area: ["host/pane-ops", "host/terminal", "host/permissions", "sdk/pgap"]
tags: ["v1", "app-terminal", "permissions", "terminal"]
---

Define the linked app-terminal contract before marketplace-style apps can drive terminals.

## Why

This is broader than paired pane close behavior. It needs to settle how apps obtain terminals, how `terminal.bindings` maps to permission prompts, how command preview and arbitrary command execution work, how lifecycle/visual grouping behaves, and how directory handoff flows avoid invisible PTY writes.

## Follow-ups

File Explorer directory handoff (#2145 / stint `0113`) consumes this contract rather than inventing a parallel cwd-sync path.

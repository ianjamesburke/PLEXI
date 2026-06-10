---
id: "0146"
title: "App authoring: CLI-backed app contract"
status: backlog
sprint: "s2"
estimate: 8h
blocked_by: ["0008"]
blocked_by_gh: []
gh_issue: []
area: ["cli/commands", "host/terminal", "sdk/pgap"]
tags: ["v1", "app-authoring", "cli-renderer", "terminal"]
---

Finalize the contract for CLI-backed Plexi apps: apps opened through the Plexi Open CLI path where the CLI owns its own Plexi UI generation and backend process, and Plexi runs the command in a background terminal/process lane.

## Why

Generated CLI apps need one obvious path before marketplace packaging and renderer hardening. The host must know how to launch, supervise, route UI updates, expose logs, request permissions, and close/restart these apps without treating arbitrary terminal subprocesses as trusted PGAP apps.

## Scope

- Define the `plexi app open --cli` app lifecycle: launch, ready state, reload, close, crash, and restart.
- Decide what runs in the background terminal/process lane and what is surfaced as the Plexi app pane.
- Specify how CLI-backed apps generate UI descriptors or frames, how Plexi caches them, and how stale descriptors are invalidated.
- Define permission prompts for command execution, filesystem access, network access, and app-to-terminal control.
- Define logging and inspection behavior so `pane info`, `pane list`, and host logs identify the backing command.
- Make the path channel-agnostic across alpha, beta, main, and PR builds.

## Blocks

- CLI renderer hardening (`0062`) should harden the implementation after this contract is settled.
- Third-party apps that generate their own Plexi UI from a CLI should block on this task.

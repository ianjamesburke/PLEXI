---
id: "0064"
title: "v1 cleanup: grouped CLI help"
status: todo
estimate: "4h"
sprint: "s11"
blocked_by:
  - 147
gh_issue:
  - "1826"
area:
  - "cli/commands"
tags:
  - "v1"
  - "cleanup"
  - "cli"
---


Group top-level `plexi --help` subcommands into Workspace, Apps, Panes, and System sections so the CLI surface is easier to scan.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

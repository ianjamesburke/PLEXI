---
id: "0163"
title: "notes: host triage surface — actions.toml routing, Cmd+Shift+0, picker inbox, CLI"
status: backlog
estimate: "10h"
sprint: "s9"
blocked_by:
  - "0162"
gh_issue:
  - "2194"
area:
  - "ui/overlays"
  - "cli/commands"
  - "host/config"
tags:
  - "notes"
---

Host triage overlay over `notes/inbox/`: one note at a time, digit-key actions from `<config_dir>/notes/actions.toml` (tokens resolved from stamped frontmatter, workspace filters), source + open-pane badges, trash-not-delete with editor banner. Cmd+Shift+0 direct entry; Cmd+O picker gains Inbox section. CLI: `plexi note`, `plexi notes inbox`, `plexi notes process`. Full spec in GH #2194.

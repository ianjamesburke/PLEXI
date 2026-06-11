---
id: "0152"
title: "Markdown: inject PlexiColors into CommonMarkViewer visuals"
status: backlog
estimate: "3h"
sprint: "s7"
blocked_by: []
gh_issue:
  - "2167"
area:
  - "ui/widgets"
  - "apps/github-issues"
tags:
  - "markdown"
  - "theming"
---

Override `extreme_bg_color`, `hyperlink_color`, and `faint_bg_color` on the child `Ui` visuals inside `RenderCommand::Markdown` (`src/process_app/render.rs:935`) so code block backgrounds use `colors.bg_active`, links use `colors.accent`, and blockquotes are tinted with `colors.bg_hover`. Also set the same three fields in `src/ui/theme.rs:setup_style` globally.

Two files, ~6 lines of change. Quick win for visual consistency across all Plexi themes.

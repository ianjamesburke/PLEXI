---
id: "0188"
title: "Marketplace: Claude Code marketplace approval for Plexi skill"
status: backlog
estimate: "4h"
sprint: "s32"
blocked_by:
  - 0183
gh_issue: []
area:
  - "docs"
  - "marketplace"
tags:
  - "v2"
  - "marketplace"
  - "skills"
  - "claude-code"
---

Prepare and submit Plexi's Claude Code plugin/skill for marketplace review once the local Plexi skill packaging story is real enough to dogfood.

## Why

Plexi should be discoverable where agent users already look for Claude Code workflows. A marketplace listing gives third-party app authors a low-friction way to install the Plexi-specific Claude Code skill, learn the app packaging conventions, and route work through the same agent workflows we use internally.

This is intentionally v2/backlog work: the listing should describe a working, maintained Plexi skill/plugin, not a speculative integration.

## Done When

- The Plexi Claude Code plugin/skill has a validated `.claude-plugin/plugin.json` manifest and minimal token-cost footprint.
- The plugin contents are reviewed for public-safe instructions, correct `plexiapp.com` links, and no repo-private operational assumptions.
- `claude plugin validate` passes locally against the distributable plugin directory.
- Marketplace submission materials are prepared: name, description, repository/source URL, version, install instructions, screenshots or docs links if required.
- The Claude Code marketplace/community approval request is submitted, and the resulting status/link is recorded in this task or a linked issue.

## Gotchas

- Do not submit before `0183` makes Plexi skill activation semantics clear enough to explain.
- Keep this as an approval/listing task, not a new plugin runtime implementation task.
- Use official Claude Code plugin docs at submission time; marketplace requirements may change.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/prm/marketplace-hosted.md`
- `skills/plexi-cli/SKILL.md`
- Claude Code plugin docs: https://code.claude.com/docs/en/plugins
- Claude Code plugin marketplace docs: https://code.claude.com/docs/en/plugin-marketplaces

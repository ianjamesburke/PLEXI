---
id: "0114"
title: "v2 PGAP: MCPUI compatibility research"
status: backlog
sprint: "s26"
estimate: 8h
blocked_by:
  - 30
  - 31
gh_issue: ["2056"]
area: ["sdk/pgap", "apps/mcp-renderer"]
tags: ["v2", "pgap", "mcpui"]
---

Explore MCPUI compatibility so Plexi apps can interoperate with MCP Apps hosts and Plexi can host MCPUI apps without compromising PGAP boundaries.

## Note

When `plexi app open` grows an MCP-specific flag/path, that path should use MCPUI directly. Do not force MCPUI apps through PGAP or treat the MCP flag as a variant of the CLI renderer path.

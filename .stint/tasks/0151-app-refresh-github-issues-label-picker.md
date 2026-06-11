---
id: "0151"
title: "App refresh: GitHub Issues label picker"
status: backlog
sprint: "s7"
estimate: 6h
blocked_by:
  - 36
gh_issue: ["2164"]
area: ["apps/github-issues"]
tags: ["apps", "app-refresh", "github", "labels"]
---

Add a keyboard label picker and smarter row label chips to the GitHub Issues app.

## Why

Issues with many labels need an explicit way to choose filters without overcrowding every row.

## Notes

- Current rows show only the first two GitHub-returned labels.
- Keep `f` as the selected-issue label cycle.
- Prefer an app-local text/fuzzy picker before changing host protocol.

## References

- GitHub issue #2164
- Follows task 0036 / issue #2110

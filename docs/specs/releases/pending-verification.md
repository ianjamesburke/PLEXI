# Pending Human Verification

Items merged to alpha that haven't yet been visually/manually confirmed by the user. As the user works through them, tick the box. If a check fails, the orchestrator runs `git revert <sha>` on alpha and files a regression issue.

Newest at the top. Completed items get archived to `DEV_LOG.md` as part of the next release-level verification gate.

## Pending

- [ ] **2026-04-25 — PR #324 (`410cb02`) — orchestration contract update.** Docs-only. Sanity check: open `Plexi Alpha.app`, launch any pane, run an existing app (snake/todo/wikipedia), confirm it renders normally. No functional behavior changed.
- [ ] **2026-04-25 — PR #323 (`2c309a0`) — v3.1–v3.5 roadmap + orchestration spec.** Docs-only. Sanity check: same as above. No functional behavior changed.

## Done
*Items move here once verified, then to DEV_LOG at release-gate time.*

# Pending Human Verification

Items merged to alpha that haven't yet been visually/manually confirmed by the user. As the user works through them, tick the box. If a check fails, the orchestrator runs `git revert <sha>` on alpha and files a regression issue.

Newest at the top. Completed items get archived to `DEV_LOG.md` as part of the next release-level verification gate.

## Pending

- [ ] **2026-04-24 — PR #315 (`9085ca8`) — bounded paint + Scrollable + chrome backdrops.** Open commit-graph at a repo with a 50+ commit week. Scroll. No green pixel bleed under FooterKeys. AppBar in any app is fully opaque (try wikipedia or commit-graph). Resize a pane while content renders; no paint outside pane bounds. (Backfilled — discovered during 2026-04-25 audit; #314 was orphan-open until then.)
- [ ] **2026-04-24 — `7c30629` — single-source pane_rect refactor.** Open the geometry-test app (or any 4-deep nested split). Every renderer call site shows correct `pane_rect`. No drift on parent resize. (Backfilled — discovered during 2026-04-25 audit; #317 was orphan-open until then.)
- [ ] **2026-04-23 — PR #313 (`fedfacc`) — host-measured text layout (Badge/KeyChip/MeasureText).** Verify badge pills are correctly sized in commit-graph and elsewhere. Key chips show as styled chips (not plain text). Truncation cuts at the right character count. (Backfilled — #312 was orphan-open until 2026-04-25.)
- [ ] **2026-04-25 — PR #324 (`410cb02`) — orchestration contract update.** Docs-only. Sanity check: open `Plexi Alpha.app`, launch any pane, run an existing app (snake/todo/wikipedia), confirm it renders normally.
- [ ] **2026-04-25 — PR #323 (`2c309a0`) — v3.1–v3.5 roadmap + orchestration spec.** Docs-only. Sanity check: same as above.

## Done
*Items move here once verified, then to DEV_LOG at release-gate time.*

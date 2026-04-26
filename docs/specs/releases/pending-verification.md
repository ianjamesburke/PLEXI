# Pending Human Verification

Items merged to alpha that haven't yet been visually/manually confirmed by the user. As the user works through them, tick the box. If a check fails, the orchestrator runs `git revert <sha>` on alpha and files a regression issue.

Newest at the top. Completed items get archived to `DEV_LOG.md` as part of the next release-level verification gate.

## POC test app gaps (retroactive — pre-rule features)

Items that shipped before the "every user-visible feature ships a POC app" rule was codified. Each needs a small `examples/<feature>-test/` follow-up so the user can verify without manually setting up scaffolding:

- [ ] **#316 lifecycle pill** — needs `examples/lifecycle-tester/` with one-click buttons: "crash now" (raise), "hang now" (infinite loop), "spam malformed JSON", "exit cleanly".
- [ ] **#287 directory-scoped registry** — needs `examples/.plexi-registry-tester/` (or a docs page in `docs/`) showing how to drop a manifest into a workspace's `.plexi/apps/` and have it appear. Probably just docs + a sample manifest, not a runtime app.
- [ ] **#322 workspace secret routing** *(in flight — sub-agent will not include the POC since the brief predates the rule)* — needs `examples/secrets-routing-tester/` post-merge: declares `[secrets] FOO_KEY = required` in its manifest, has a "fetch" button that calls `ctx.secret('FOO_KEY')` and displays the resolved value plus which Keychain entry was used.

## Pending

- [ ] **2026-04-25 — PR #327 (`e3c7f06`) — directory-scoped registry (#287).** Create `~/work-test/.plexi/apps/test-app/manifest.toml` with a valid manifest. Launch Plexi Alpha from `~/work-test/`. test-app appears in command palette. Same id present globally — local wins; an `info` line `shadows global entry from …` appears in `~/.plexi-alpha/plexi.log`. Launch from `~/`; local app gone, global apps still work. Drop `.plexi/agents/test-agent/manifest.toml`; agent is discoverable.
- [ ] **2026-04-25 — PR #326 (`39e2f5d`) — TextInput primitive (#283).** Open `Plexi Alpha.app`, run the `backlog` app, press `n`. A "New backlog item" overlay appears with a text input. Type a name, press Enter — a new markdown stub appears in the list. Resize the pane while typing — content + cursor survive. Spawn a second backlog pane; each pane's input buffer is independent.
- [ ] **2026-04-25 — PR #325 (`32695ed`) — observable app lifecycle pill (#316).** Open `Plexi Alpha.app` and run snake — no pill should be visible (Running is invisible). Then drop in a crashy app: copy `examples/snake/snake.py` into a new app dir, replace its body with `raise RuntimeError("boom")`, install with `just install-alpha`, spawn it. Within 1s a red `crashed` pill appears top-right; click it to toggle the stderr traceback overlay. With a healthy app running, `kill -9 $(pgrep -f <app>.py)` from a terminal — pill goes red within 1s, host stays alive. (Hung + protocol_error states are best-effort to verify; skip if low-value.)
- [ ] **2026-04-24 — PR #315 (`9085ca8`) — bounded paint + Scrollable + chrome backdrops.** Open commit-graph at a repo with a 50+ commit week. Scroll. No green pixel bleed under FooterKeys. AppBar in any app is fully opaque (try wikipedia or commit-graph). Resize a pane while content renders; no paint outside pane bounds. (Backfilled — discovered during 2026-04-25 audit; #314 was orphan-open until then.)
- [ ] **2026-04-24 — `7c30629` — single-source pane_rect refactor.** Open the geometry-test app (or any 4-deep nested split). Every renderer call site shows correct `pane_rect`. No drift on parent resize. (Backfilled — discovered during 2026-04-25 audit; #317 was orphan-open until then.)
- [ ] **2026-04-23 — PR #313 (`fedfacc`) — host-measured text layout (Badge/KeyChip/MeasureText).** Verify badge pills are correctly sized in commit-graph and elsewhere. Key chips show as styled chips (not plain text). Truncation cuts at the right character count. (Backfilled — #312 was orphan-open until 2026-04-25.)
- [ ] **2026-04-25 — PR #324 (`410cb02`) — orchestration contract update.** Docs-only. Sanity check: open `Plexi Alpha.app`, launch any pane, run an existing app (snake/todo/wikipedia), confirm it renders normally.
- [ ] **2026-04-25 — PR #323 (`2c309a0`) — v3.1–v3.5 roadmap + orchestration spec.** Docs-only. Sanity check: same as above.

## Done
*Items move here once verified, then to DEV_LOG at release-gate time.*

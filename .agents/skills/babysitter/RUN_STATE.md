# Babysitter run state — overwritten at each merge boundary

updated: 2026-07-29T11:37Z (babysitter-17, Claude cm, pane 466)
mode: stints 0627 0624 0629 0608 0610 0611 0607 0605 0600 0604 0630 0615 0616 0617 0577
auto_merge: yes (RUN_CONFIG [authorization].auto_merge = true)

merged: batch1 (0561+0563+0564) -> PR #2514. batch2 (0562) -> PR #2515. batch3 (0632) -> PR #2516. batch4 (0618+0619+0620+0621) -> PR #2517 as e89b796d, MERGED 11:04Z; all four verified done. batch4 = brief 09:58Z -> merge 11:04Z (~66m), 2 tester rounds (1 wasted), 0 fix rounds.
active: none. worker-4 (467), tester-8 (468), tester-9 (469) all closed or closing. Working tree CLEAN as of 135e5fa5 (babysitter SKILL.md lesson committed).

next: batch5 = 0627 SOLO, LARGE tier (col) -- it is a p1 live regression and the repro is the hard part (p1 live fullscreen-terminal input regression; worker must add a failing HostHarness fullscreen-terminal key-events-to-PTY-bytes repro FIRST, with tiled terminal and fullscreen editor as passing controls). Then 0624 solo (host-owned pane heartbeat, task-body tests). Then 0629. Then 0608+0610+0611 (0609 may join ONLY if that worker proves it separable and it does not widen the batch). Then 0607. Then 0605+0600+0604+0630 (0630 = pane id badge missing when a window has only one pane, p2/S, repro-first). Then 0615+0616+0617. Then 0577.
excluded: 0601 0439 0602 0596 0598 0622
exceptions: Park any batch at 3 fix rounds with a follow-up stint and move on. For 0577: implement, gate, open a green PR, then park UNMERGED for Ian's attended review regardless of auto_merge.

run_hazard: RESOLVED for compile — 0632 landed, so just scene-live builds again. STILL OPEN for pixels: scene-live's live backend skips its optional screenshot and plexi host screenshot times out against an occluded host, so visual gates still cannot emit a PNG. Filed as stint 0633 (P1/S, infra/testing, s1). Until 0633 lands, testers log a missing visual gate as a documented carve-out in HUMAN_CHECKS.md, never as a product FAIL.
standing_rule: timing/wall-clock/perf failures under fleet load are a known class (stint 0629) — one quiet-baseline rerun before chasing; if it passes quiet, cite 0629 and move on.

head_lessons: SKILL.md, HUMAN_CHECKS.md and RUN_STATE.md are TRACKED. Commit them BEFORE handing off a merge or the dirty-tree gate blocks just merge-pr and the sync step destroys uncommitted edits. **Two SKILL.md lesson promotions from batch3 are still UNCOMMITTED — the batch4 merge boundary MUST commit them.** Never let an armed background watch replace reading the actor's own reply. merge-cleanup/merge-close-stints take positional args: PR BRANCH / PR STINTS...
lessons: live beta lacks pane send --submit, pane new --agent, pane status, AND pane slot wait — use the compatibility fallback. A fallback-spawned pane does NOT get the alias bypass, so it WILL hit permission prompts; brief every pane to never use rm -rf (fresh unique scratch dirs instead) and keep a ~5min cadence until the PR opens. Codex rejects slash-form skill invocation; worker briefs name the Worker Mode contract in prose. The codex footer's "medium" is reasoning effort, NOT model tier — col IS the large tier. Never chain a pane close behind file writes in one bash call — it hangs and the writes are lost.

# Babysitter run state — overwritten at each merge boundary

updated: 2026-07-29T14:30Z (babysitter-19, Claude cm, pane 473)
mode: stints 0629 0608 0610 0611 0607 0605 0600 0604 0630 0615 0616 0617 0577
auto_merge: yes (RUN_CONFIG [authorization].auto_merge = true)

merged: batch1 (0561+0563+0564) -> PR #2514. batch2 (0562) -> #2515. batch3 (0632) -> #2516.
batch4 (0618+0619+0620+0621) -> #2517 as e89b796d. batch5 (0627) -> #2518 as 5c67bc5b. batch6 (0624) -> #2519 as ef8f4905,
MERGED 12:26Z; 0624 verified done. batch6 = brief 13:05Z -> merge 12:26Z UTC (~55m),
1 tester round, 0 fix rounds, no human check needed (all 7 scope items driven LIVE).
batch5 (0627) merged 11:42Z, 1 tester round. Chore commit dcd525d9. Stint 0634 filed.
active: none. worker-6 (474) and tester-11 (475) closed. Tree clean at ef8f4905.

next: batch7 = 0629 SOLO. Then
0608+0610+0611 (0609 may join ONLY if that worker proves it separable and it does not
widen the batch). Then 0607. Then 0605+0600+0604+0630 (0630 = pane id badge missing when
a window has only one pane, p2/S, repro-first). Then 0615+0616+0617. Then 0577.
excluded: 0601 0439 0602 0596 0598 0622
exceptions: Park any batch at 3 fix rounds with a follow-up stint and move on. For 0577:
implement, gate, open a green PR, then park UNMERGED for Ian's attended review regardless
of auto_merge.

run_hazard: scene-live compiles again (0632) but its live backend still skips the optional
screenshot and plexi host screenshot times out against an occluded host, so visual gates
still cannot emit a PNG. Filed as stint 0633. Until it lands, testers log a missing visual
gate as a documented HUMAN_CHECKS.md carve-out, never a product FAIL.
run_hazard_2 (NEW, batch5): NO CLI VERB can fullscreen/zoom a pane, and `pane key <id>
cmd+enter` writes literal PTY bytes rather than triggering the host shortcut. Any
fullscreen-conjunction behavior is therefore harness-only-testable — brief testers
accordingly and route it to HUMAN_CHECKS.md instead of demanding a live drive. Stint 0634.
standing_rule: timing/wall-clock/perf failures under fleet load are a known class (stint
0629) — one quiet-baseline rerun before chasing; if it passes quiet, cite 0629 and move on.

head_lessons: SKILL.md, HUMAN_CHECKS.md, RUN_STATE.md are TRACKED — commit them BEFORE
briefing the next worker (Worker Mode preflight hard-stops on a dirty root) and before any
merge (the sync step destroys uncommitted edits). LOG.md is gitignored. Never chain a pane
close behind file writes in one bash call. Before merging, check that the ONE behavior the
stint is about appears BY NAME in the tester's LIVE evidence list — a PASS summary can
silently substitute harness evidence for the live evidence the brief demanded (caught live
on #2518 with one clarifying question; that question is not a fix round).
lessons: live beta lacks pane send --submit, pane new --agent, pane status, AND pane slot
wait — use the compatibility fallback. A fallback-spawned pane does NOT get the alias
bypass, so brief every pane to never use rm -rf (fresh unique scratch dirs instead). After
a fallback-form brief send, press enter and CONFIRM "esc to interrupt" appears — the first
enter often only settles the paste. A head bash loop that waits for pane-idle trips on the
momentary idle between Codex tool calls; require idle twice ~20s apart, or use the slot.
Codex rejects slash-form skill invocation; worker briefs name the Worker Mode contract in
prose. The codex footer's "medium" is reasoning effort, NOT model tier — col IS large.
new_capability: 0624 landed on alpha (ef8f4905) — `plexi pane heartbeat <id> --every <dur>
--text "cycle"` is host-owned and idle-gated. NOT yet in the live beta binary; usable once
beta rebuilds. It replaces the operator-as-clock half of RUN_CONFIG [cadence].
quota: 88% of weekly Claude limit used as of 09:43Z, resets Jul 30 2pm America/Detroit.

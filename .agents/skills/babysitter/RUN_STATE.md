# Babysitter run state — overwritten at each merge boundary

updated: 2026-07-29T17:35Z (babysitter-20, Claude cm, pane 476)
mode: stints 0608 0610 0611 0607 0605 0600 0604 0630 0615 0616 0617 0577
auto_merge: yes (RUN_CONFIG [authorization].auto_merge = true)

merged: batch1 (0561+0563+0564) -> #2514. batch2 (0562) -> #2515. batch3 (0632) -> #2516.
batch4 (0618-0621) -> #2517. batch5 (0627) -> #2518. batch6 (0624) -> #2519.
batch7 (0629) -> #2520 as 955c9b2f, MERGED 14:17:51Z, stint 0629 verified done 14:18:16Z.
batch7 = brief 14:45Z -> merge 14:18Z (~2h35m across 3 tester rounds, 2 fix rounds).
active: none. worker-7 (477) still open pending the follow-up stint; testers 478/479/480 closed.

RUN BLOCKED ON DISK — read before briefing anything:
/System/Volumes/Data at 423 MiB free of 926 GiB (100%). pr-install is a multi-GB Rust build,
so NO tester round can run and batch8 CANNOT start. I reaped ~19 GB of agent scratch
checkouts under /tmp and the free figure did not move, so the pressure is not only ours.
Known large: PLEXI/target 33G, ~/Library/Caches 27G, ~/.cargo 2.6G, 12 stale ~/.plexi-pr-*
profiles (oldest 16 Jun). Those are Ian's call, not an agent's. Surfaced to Ian 10:16 local
with a recommendation to cargo clean the 33G target first. DO NOT RESUME until he clears it.

CAUSE + standing rule change: the "never rm -rf, leave scratch behind" clause in every brief
is what filled the disk (four agent scratch checkouts alone = 19 GB). That clause exists to
stop Codex permission stalls and must NOT be dropped. Instead THE HEAD now reaps: at every
merge boundary, after reading the verdicts, delete this batch's /tmp/plexi-* scratch dirs
yourself. Agents still never rm -rf.

next: batch8 = 0608+0610+0611 (0609 may join ONLY if that worker proves it separable and it
does not widen the batch). Then 0607. Then 0605+0600+0604+0630 (0630 = pane id badge missing
when a window has only one pane, p2/S, repro-first). Then 0615+0616+0617. Then 0577.
excluded: 0601 0439 0602 0596 0598 0622
exceptions: park any batch at 3 fix rounds with a follow-up stint and move on. For 0577:
implement, gate, open a green PR, then park UNMERGED for Ian's attended review regardless
of auto_merge.

run_hazard: scene-live compiles again (0632) but its live backend still skips the optional
screenshot and plexi host screenshot times out against an occluded host, so visual gates
cannot emit a PNG. Stint 0633. Until it lands, testers log a missing visual gate as a
documented HUMAN_CHECKS.md carve-out, never a product FAIL.
run_hazard_2: NO CLI VERB can fullscreen/zoom a pane, and pane key <id> cmd+enter writes
literal PTY bytes rather than triggering the host shortcut. Fullscreen behavior is
harness-only-testable — route it to HUMAN_CHECKS.md, never demand a live drive. Stint 0634.
standing_rule: timing/perf failures under fleet load are a known class (0629, now landed) —
one quiet-baseline rerun before chasing; if it passes quiet, cite 0629 and move on.

head_lessons (batch7, NEW):
- THE COMMIT-FIRST STEP IS NOT OPTIONAL AND WORKERS SKIP IT. I told worker-7 to commit
  RUN_STATE.md before just merge-pr; it went straight to the merge and the sync step
  silently reverted my whole baton to the batch6 version. Next time: verify the
  chore(babysitter) commit EXISTS in git log before authorizing the merge command.
- I caused a false FAIL. My fix-round brief asked for a STRUCTURAL/type-enforced boundary as
  an aspiration; tester-14 turned that into a pass criterion and failed the PR for it, though
  it was never in 0629's Done-When. Aspirational language in a fix brief becomes a gate.
  State the real criterion explicitly, or say "if cheap, consider X — not a gate."
  Overruled on the record and merged; follow-up stint for capability-token enforcement.
- A tester hit "No space left on device" mid-round and reported it instead of guessing.
  Trust that immediately as infrastructure, never treat it as a product finding.
- Reading a 6-line pane tail made a working pane look idle for a cycle. Read ~20 lines
  before concluding a send did not land.
- codex /model <name> echoes but the footer keeps the old model — mid-run escalation is
  unverifiable. Pick the tier at spawn; do not thrash on /model.

lessons (carried): live beta lacks pane send --submit, pane new --agent, pane status, AND
pane slot wait — use the compatibility fallback. Fallback-spawned panes do NOT get the alias
bypass. After a fallback-form brief send, press enter and CONFIRM "esc to interrupt" appears;
the first enter often only settles the paste. A head bash loop waiting for pane-idle trips on
the momentary idle between Codex tool calls — require idle twice ~20s apart, or use the slot.
Codex rejects slash-form skill invocation; worker briefs name the Worker Mode contract in
prose. The codex footer's "medium" is reasoning effort, NOT model tier — col IS large.
new_capability: 0624 (ef8f4905) — plexi pane heartbeat <id> --every <dur> --text "cycle",
host-owned and idle-gated. Not in the live beta binary yet; usable once beta rebuilds.
quota: 88% of weekly Claude limit used as of 09:43Z Jul 29, resets Jul 30 2pm America/Detroit.

# Babysitter Run Log

Workflow telemetry for the babysitter skill. Append-only, UTC timestamps, terse lines.
Per-run: worker/tester timings, attempt counts, verdicts. Highest-value entries: workflow friction — forgotten brief clauses, redundant tool calls, thrash loops, gotchas not yet in SKILL.md.
End each sprint block with a `### Sprint recap`: what landed, timing/tries table, learnings, concrete workflow-streamlining suggestions. Promote validated suggestions into SKILL.md and note it here.

---

## 2026-07-21 — sprint s3 (Notes editor), sequential walk, auto-merge on PASS
- 19:27 worker-1 (pane 125) briefed: stint 0317 (extract Ferrite-derived shared editor core), c/fable, high tier
- 16:47 PR #2462 open (stint 0317, ~1h20m brief->PR), CI green (CodeRabbit pass, check-cli-docs pass)
- 16:47 note: 0317 is editor-core-only, no pane wiring (0474 owns integration); tester validates the egui harness surface, not a live pane
- 16:49 tester-1 (pane 126) briefed: validate PR #2462 via editor harness surface, co/gpt-5.6-terra
- 17:05 tester-1: FAIL (attempt 1) — no-live-surface artifact, not a defect (0317 harness is cfg(test)-only, integration deferred to 0474); user approved merge
- 17:05 worker-1 running just merge-pr 2462
- 21:12 PR #2462 MERGED, stint 0317 done (1 round, ~4h25m brief->merge incl. long L-sized port). worker-1 retired.
- friction: worker committed babysitter LOG.md directly to alpha during merge cleanup (6a2aa664) — harmless but off-protocol; LOG edits are babysitter-owned
- 21:14 worker-2 (pane 127) briefed: stint 0474 (swap Notes onto shared editor core), c/fable, high tier — first task with a live Notes surface
- 18:17 PR #2463 open (stint 0474, ~1h03m brief->PR), CI green (Rust-only, CodeRabbit pass). Live Notes surface present -> real tester drive.
- 18:18 tester-2 (pane 128) briefed: live-drive Notes editor on PR #2463, co/gpt-5.6-terra
- 18:34 tester-2: FAIL (attempt 1, 9m) — 2 real bugs: (1) cmd+v paste no-op, (2) pane click doesn't place caret (blocks click/drag select). Typing/delete/movement/shift-select/undo-redo/grapheme/autosave all PASS live. -> fix round on worker-2 (already fable)
- 18:38 worker-2 compacted + fix brief sent for PR #2463 (paste no-op + click-caret no-op), root-cause-first framing (input dispositions not routed to transaction path)
- 18:5x INCIDENT: worker-2's full-suite gate on PR #2463 spawned a test binary (deps/plexi-<hash>) that ballooned to 22 GB in ~9 min, driving the machine into swap. Killed PID 95657; mem returned to 70% free. Interrupted worker-2, re-briefed to isolate the ballooning test under `ulimit -v 8388608` single-thread and root-cause the unbounded alloc/loop (likely the paste/click-caret fix).
- FRICTION -> promote: worker test-suite runs have no memory ceiling; a runaway takes down the whole machine. Every worker/fix brief should wrap `cargo test` in `( ulimit -v 8388608; ... )` so a runaway self-aborts. Candidate for SKILL.md worker-brief boilerplate.

- 02:59 tester-3: FAIL round2 (PR#2463) — paste still no-op (disposition: text_input, identical pre-fix string), click still pins caret at doc-end, cmd+c also broke. Worker fix did not change observed behavior in installed build.

- 03:18 worker-2 fix-round2 done: root cause = tester-3 wrong-tree install (pr-install built tree WITHOUT PR head commit -> drove unfixed alpha; byte-identical symptom was the tell). Code was correct at 060e5eca. Shipped 70698237 (pr-install guard: hard-error if tree lacks PR head) + e7c56131 (accurate key-delivery msg). Suite 1641 passed. FRICTION->promote: pr-install could silently build wrong tree; guard now prevents it.

- 03:35 tester-4: PASS (attempt 3, correct-tree install). Auto-merging PR#2463 (stint 0474).

- 03:36 MERGED PR#2463 (stint 0474) — SHA on alpha, stint done. Batch1 (0317+0474) complete: 3 tester rounds (t3 false-FAIL wrong-tree install, t4 PASS). Total ~ multi-hr, 2 fix rounds.

- 03:37 worker-3 briefed: stints 0318+0475+0476 (editor integration / md transactions / live preview), c/fable, one PR. Batch2 start.

- 05:36 worker-3 PR #2464 open (batch2: 0318+0475+0476). Auto-resumed after background 0476 sub-agent, no stall. Checks green (CodeRabbit + check-cli-docs). ~2h impl (3 sub-agents, heavy tokens, no runaway). Spawning tester-5.

- 05:37 tester-5 briefed for PR #2464 (install+drive 0318/0475/0476 + paste/click regression smoke).

- 05:53 tester-5: PASS (attempt 1) — 0318/0475/0476 all verified live, paste/click no regression. Non-blocker found: pane send misparses leading-dash text as flag (clap), broke scene not feature -> follow-up. Auto-merging #2464.

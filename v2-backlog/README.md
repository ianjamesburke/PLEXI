# v2 Backlog (parked)

This directory holds the **v2 phase** of the Plexi plan, parked out of the active
`.stint/` operating board. It is intentionally invisible to the `stint` CLI.

## Why this exists

`stint` has no native "phase" concept — only sprints, blockers, and gates. While
v2 tasks lived in the active board, they broke two things at once:

- **`stint next`** offered v2 work as top picks (it orders ready tasks by id, not
  sprint, and area-dedup floated v2 above congested v1 areas).
- **The bottleneck signal** was destroyed: a single `v2-after-v1` gate made every
  v2 task inherit-block on `0030` ("ship v1"), so the bottleneck was always the
  useless tautology "ship v1 blocks 81 tasks."

Parking v2 here makes the live board v1-only: `stint next` cues v1 in order, and
the bottleneck points at the largest *real* v1 dependency chain.

## Contents

- `tasks/` — all 83 v2 task files (81 open + 2 done: `0085`, `0161`).
- `sprints/` — the pure-v2 sprints `s15`–`s30`.
- `gate/v2-after-v1.md` — the gate that holds v2 behind v1 release (`0030`, `0031`).

Mixed sprints `s7` and `s8` kept their v1 tasks in the live board; their single v2
members (`0058`, `0037`) moved here. On reintroduction, add them back to those
sprint files.

## Reintroduction (when v1 ships — i.e. task `0030` is done)

```sh
cd <repo root>
git mv v2-backlog/tasks/*.md   .stint/tasks/
git mv v2-backlog/sprints/*.md .stint/sprints/
git mv v2-backlog/gate/v2-after-v1.md .stint/gates/v2-after-v1.md
# restore the two mixed-sprint members:
printf -- '- 0058\n' >> .stint/sprints/s7.md
printf -- '- 0037\n' >> .stint/sprints/s8.md
rmdir v2-backlog/tasks v2-backlog/sprints v2-backlog/gate v2-backlog 2>/dev/null
stint check
```

After v1 ships, the gate's anchor (`0030`) is already done, so reintroduced v2
tasks become claimable immediately and `stint next` flows into the v2 phase.

## Invariants verified at park time (2026-06-12)

- No v1 task is `blocked_by` a v2 task — parking breaks no live dependency.
- All v2 cross-references (v2→v2 and v2→done-v1) are preserved inside the files
  and resolve again on reintroduction.

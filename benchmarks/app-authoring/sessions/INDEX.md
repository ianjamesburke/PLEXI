# Session index

One row per captured session. Stint 0215 fills this with the baseline sweep.

| Session | Fixture | Difficulty | Channel | Mode | Outcome |
|---|---|---|---|---|---|
| 20260702T160950Z_counter_43fabc | counter | easy | e2e | dry-run | plan-only (reference example) |

The `dry-run` reference example proves the capture format and runner plumbing
end-to-end without a live host. Its `plan.json` is the exact command sequence a
live run executes. Replace it (or add alongside) with a live session once the
`e2e` channel is installed and a child agent with credentials is available.

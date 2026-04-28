# plexi-descriptor-demo

A standalone CLI script that demonstrates the `--plexi` descriptor format from
issue [#188](https://github.com/ianjamesburke/PLEXI/issues/188).

**This is not a Plexi app.** It is a regular Python CLI that opts in to the
auto-UI standard by responding to `--plexi` with a JSON descriptor matching
`schemas/plexi-descriptor-schema.json` at the repo root.

## Try it

```bash
# Emit the descriptor JSON straight to stdout.
python plexi_descriptor_demo.py --plexi

# Round-trip through the host parser. Pretty-prints a summary and exits 0
# on a valid descriptor; surfaces the broken field path on a malformed one.
plexi-alpha descriptor probe python plexi_descriptor_demo.py
```

## What the descriptor expresses

The script's `DESCRIPTOR` dict mirrors the example from the issue body:

- Three top-level commands (`run`, `status`, `project`).
- `run` is a `form` UI: positional `brief` (string, required) + a `--test-mode`
  flag (bool). It declares `writes: [".parallax/"]` for trust gating and
  `streaming: true` so the consumer knows stdout is progress-over-time, not a
  one-shot result.
- `project` is a subcommand group with nested `commands[]` (`new`, `list`) —
  the schema is recursive, so `git`-style multilevel CLIs work.
- `live_state` tells Plexi to poll `.parallax/manifest.yaml` every 1000 ms and
  parse it as YAML so the UI re-renders when the agent mutates it out of band.

## Authoring a descriptor for your own CLI

1. Read the schema at `schemas/plexi-descriptor-schema.json` (draft-07,
   strict — unknown fields fail loudly).
2. Read the proposal at `docs/specs/proposals/plexi-descriptor.md` for the
   field semantics and versioning policy.
3. Add a `--plexi` branch to your CLI's argument parser. Print the JSON.
   Exit 0.
4. Verify with `plexi-alpha descriptor probe <your-cli>`.

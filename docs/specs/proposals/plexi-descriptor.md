# `--plexi` Descriptor Format (v0)

**Status:** v0 — substrate spec for v3.5. Closes #188.
**Schema:** [`schemas/plexi-descriptor-schema.json`](../../../schemas/plexi-descriptor-schema.json)
**Parser:** [`src/plexi_descriptor.rs`](../../../src/plexi_descriptor.rs)
**Reference CLI:** [`examples/plexi-descriptor-demo/`](../../../examples/plexi-descriptor-demo/)

## Pitch

Any CLI can opt in to Plexi auto-UI by responding to `--plexi` with a JSON
descriptor of its commands, flags, and arg types. Plexi parses the descriptor
and renders a clickable interface — no app authoring, no per-CLI integration.
This is the opt-in, CLI-author-driven counterpart to `--help` parsing
(#187/#78). If `--plexi` exists, use it; otherwise fall back to crawling
`--help`.

## What this PR delivers

- The JSON Schema that defines the descriptor format (draft-07, strict
  `additionalProperties: false` everywhere).
- A serde-typed Rust parser (`crate::plexi_descriptor`) with the matching
  strictness invariants and a clear error type.
- A `plexi descriptor probe <command>` subcommand that runs `<command>
  --plexi`, parses the result, and prints a summary. This is the reference
  consumer; the full auto-UI renderer ships in #78.
- A standalone demo CLI in `examples/plexi-descriptor-demo/` showing how a
  CLI author wires up `--plexi`.

What this PR does **not** deliver: the auto-UI renderer (#78), the wrapper
registry for CLIs that don't natively emit `--plexi` (#321), live-state
polling, the `--help` fallback (#187), capability-preview integration, or
SDK helpers (`plexi_cli.from_argparse(...)`).

## Schema

The full schema lives in [`schemas/plexi-descriptor-schema.json`](../../../schemas/plexi-descriptor-schema.json).
Annotated abridged shape:

```jsonc
{
  // Format-version of the descriptor itself, NOT the CLI version. v0.x
  // parsers reject v1+ loudly. Bump major when the schema breaks.
  "plexi_version": "0.1",

  // Human-friendly CLI name + the CLI's own version + a one-line description.
  "name": "parallax",
  "version": "0.1.0",
  "description": "Video agent pipeline CLI",

  // Optional emoji/icon glyph rendered next to the CLI name.
  "icon": "🎬",

  // Render hint when nothing is selected. Shared enum with command-level
  // ui_hint: "form" | "output" | "tabs" | "stream" | "list".
  "default_view": "list",

  "commands": [
    {
      "name": "run",
      "description": "Kick off a footage_edit run in cwd",
      "icon": "▶",

      // How Plexi should render this command:
      //   form   — flags-as-inputs + submit button
      //   output — execute and show stdout
      //   tabs   — group of subcommands as tabs
      //   stream — long-running process, render progress
      //   list   — browse subcommands as a list
      "ui_hint": "form",

      // Positional args, in order.
      "args": [
        {
          "name": "brief",
          "type": "string",        // string|int|float|bool|path|enum
          "required": true,
          "description": "What you want the agent to create",
          "placeholder": "western cowboy scene, 8 seconds"
        }
      ],

      // Long-form flags. Names include the leading `--`.
      "flags": [
        { "name": "--test-mode", "type": "bool", "default": false }
      ],

      // Capability hints: paths the command may write/read. Plexi can
      // surface a trust prompt before the first run.
      "writes": [".parallax/"],
      "reads": [],

      // True if stdout streams progress over time. Consumers render this as
      // a stream pane, not a one-shot result.
      "streaming": true,

      // Hint at the shape of stdout when ui_hint = "output". Free-form;
      // common values: text, json, yaml, table.
      "output_format": "yaml",

      // Recursive: a command can carry nested subcommands. `git remote add`
      // is commands → commands → commands.
      "commands": []
    }
  ],

  // Out-of-band state Plexi should watch for changes. Source is one of
  // "file" (poll a path), "socket" (unix socket), "http" (poll a URL).
  "live_state": {
    "source": "file",
    "path": ".parallax/manifest.yaml",
    "poll_ms": 1000,
    "format": "yaml"          // json|yaml|text
  }
}
```

### Field reference (terse)

| Field | Required? | Notes |
|---|---|---|
| `plexi_version` | yes | Semver `MAJOR.MINOR[.PATCH]`. v0 parser refuses v1+. |
| `name` | yes | Display name. |
| `version` | yes | CLI's own version. Used by registry cache keys. |
| `description` | no | One-line. |
| `icon` | no | Emoji or glyph. |
| `default_view` | no | Same enum as `ui_hint`. |
| `commands[]` | yes | May be empty. |
| `live_state` | no | All four sub-fields required when present. |

Per-command, `name` is the only required field. Per-arg, `name` and `type`.

### Versioning policy

`plexi_version` is the format version, not the CLI version. The contract:

- **Patch (0.1.0 → 0.1.1)** — bug-fixes to the spec text. No code change.
- **Minor (0.1 → 0.2)** — additive only. New optional fields, new enum
  variants. Old parsers can still read new descriptors (with `deny_unknown_fields`
  off for unknown optional fields — see "Forward-compat" below).
- **Major (0.x → 1.0)** — breaking. v0 parsers reject loudly with
  `UnsupportedMajorVersion`.

**Forward-compat caveat (v0):** the v0 parser uses `serde(deny_unknown_fields)`
strictly so authors get fast feedback during development. When v0.2 introduces
new optional fields, v0.1 parsers will reject them. The intended fix is to
relax to "warn-on-unknown" before the first minor bump; tracked as future work.
For now, ship descriptors at the parser's known minor.

### Arg-type vocabulary (v0)

`string | int | float | bool | path | enum`. Notably narrower than the issue
body's draft (which included `number`, `file`, `dir`, `color`, `multiselect`).
The narrower set is deliberate for v0:

- `int` + `float` are explicit; `number` is ambiguous between them.
- `path` covers both files and dirs; the consumer renders an appropriate
  picker. Splitting `file` vs `dir` adds policy without payoff at this stage.
- `color` and `multiselect` are UI conveniences that belong in v0.x as
  additive minor bumps once a real consumer (#78) needs them.

## Authoring guide

1. Read [`schemas/plexi-descriptor-schema.json`](../../../schemas/plexi-descriptor-schema.json).
   It is the source of truth. Any IDE with JSON Schema support will
   autocomplete + validate as you type.
2. Add a `--plexi` branch to your argument parser. When the flag is present,
   print the descriptor JSON to stdout and exit 0. All other invocations
   behave normally.
3. Pin `plexi_version` to the schema's current major-minor (`0.1` today).
4. Verify with `plexi-alpha descriptor probe <your-cli>`. The probe parses
   the output against the host's strict schema; broken descriptors surface
   the offending field path in stderr.

See [`examples/plexi-descriptor-demo/plexi_descriptor_demo.py`](../../../examples/plexi-descriptor-demo/plexi_descriptor_demo.py)
for an end-to-end Python example. The descriptor is a literal dict; the
`--plexi` branch is three lines (`json.dump(DESCRIPTOR, sys.stdout); print();
return 0`).

## Consumer guide (sketch — implementation in #78)

1. Resolve the binary the user wants UI for.
2. Run `<binary> --plexi`. Capture stdout.
3. Parse via `crate::plexi_descriptor::parse(&stdout)`. On
   `UnsupportedMajorVersion` or `SchemaMismatch`, fall through to:
4. (#187 / #78 territory) `<binary> --help` parsing as the inferred fallback.
5. Render based on `default_view` and per-command `ui_hint`. Commands with
   nested `commands[]` recurse — typically into a tab/list group.
6. On user submit, build the equivalent CLI string and write it to the linked
   terminal's PTY (the canvas-terminal binding from #78).
7. If `live_state` is present, poll `path` every `poll_ms` ms, parse as
   `format`, re-render.

## Why this format, not a curated registry

- **Decentralized.** Each CLI ships its own descriptor. No bottleneck.
- **Rich.** Carries metadata `--help` can't (icons, view modes, live state,
  arg types, streaming hints).
- **Universal.** Any tool that auto-generates UIs from CLIs can consume it —
  Plexi is the first, not the last.
- **Opt-in.** CLIs that don't care pay nothing; the fallback path
  (`--help` parsing in #78) covers them.

## Open questions / future work

- **Localization.** All strings are presumed English. A `lang` field at the
  top level + per-string i18n bundles is a future minor bump.
- **Capability descriptor cross-walk.** The `writes`/`reads` fields are
  hints, not enforceable claims. Aligning them with the v3 capability
  manifest grammar is a future bump.
- **Schema-relaxation strategy for forward-compat.** See "Versioning
  policy" above. Likely lands as a parser-side `lenient_unknown_fields`
  toggle plus a deprecation runway.
- **SDK helpers.** `plexi_cli.from_argparse(parser)` /
  `plexi_cli::from_clap(app)` would make adoption painless. Not v0 scope.

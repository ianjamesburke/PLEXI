# CLI Wrapper Registry (issue #321)

The middle tier between `--plexi`-native CLIs (#188) and `--help` crawl
fallback (#78). Most CLIs won't ship `--plexi` for years; crawling `--help`
is lossy. The registry is the pragmatic middle path: hand-authored
descriptors for popular CLIs, kept fresh by tooling.

## Three-tier resolution

1. **Tier 1** — the CLI itself emits a descriptor when invoked with
   `--plexi`. Fastest, always in sync with the installed binary.
2. **Tier 2** — *this proposal*. The Plexi binary consults a baked-in
   registry keyed by `(cli_name, version)`. A user-side override directory
   at `~/.plexi-<channel>/registry/` shadows the embedded copy so users can
   patch a descriptor locally without rebuilding Plexi.
3. **Tier 3** — fallback `--help` crawl, owned by issue #78.

## In-repo registry layout

```
registry/
  gh/
    2.40.0.json     # version-pinned descriptor
    latest.json     # copy of the highest version (NOT a symlink — Windows-safe)
  cargo/
    1.75.0.json
    latest.json
  npm/
    10.0.0.json
    latest.json
```

Each `*.json` validates against `schemas/plexi-descriptor-schema.json`. The
file format is identical to the `--plexi` flag output (#188) — a registry
descriptor and a native-emitted descriptor are interchangeable.

`latest.json` is a copy rather than a symlink so the layout works on
Windows and inside the `include_dir!` macro without symlink chasing. A
unit test (`embedded_registry_round_trips_through_parser`) guards against
hand-edit drift by re-parsing every shipped descriptor.

## Authoring a new descriptor

1. Pick a CLI version. Run the CLI's `--help` and explore its commands.
2. Author `registry/<cli>/<version>.json` against the descriptor schema.
   See `examples/plexi-descriptor-demo/plexi_descriptor_demo.py` for the
   shape — top-level fields are `plexi_version`, `name`, `version`,
   `description`, `icon`, `default_view`, `commands`, optional
   `live_state`. Keep the initial set of commands tight — 3–5 top-level
   commands per CLI is enough to ship.
3. Copy the file to `registry/<cli>/latest.json`.
4. Run `cargo test --bin plexi cli_registry` — the round-trip test must
   pass.
5. Verify with `plexi descriptor probe <cli>`. The summary should show
   `(via registry)`.

## Release-watcher CLI usage

`plexi registry watch [<cli>]` walks the registered CLI set (or just the
named one), comparing the locally-installed binary against its registry
entry:

- **Not installed** — skipped quietly.
- **Up to date** — installed version matches registry, and `--help`
  command set matches the descriptor.
- **Stale** — installed version > registry version. Descriptor needs a
  refresh.
- **Drift** — installed version matches, but `--help` shows commands not
  in the descriptor (or vice versa). Descriptor body is incomplete.

Output is human-readable. Machine-parseable JSON is intentionally
deferred — the cron-driven watcher is a follow-up.

## Roadmap (follow-ups)

- **Author full descriptors for the remaining 7 CLIs** — `git`, `docker`,
  `kubectl`, `brew`, `uv`, `rg`, `fzf`. One small PR per CLI.
- **Automated release-watcher cron** — wrap `plexi registry watch` in a
  GitHub Action that runs daily, sandbox-installs the new CLI, opens a PR
  with the diffed descriptor.
- **Public CDN at `registry.plexi.app`** — host the registry outside the
  binary so updates don't require a Plexi rebuild. Today the registry
  ships baked into the binary; CDN move is a no-op for descriptor
  authors.
- **Per-platform variations** — same CLI may expose different commands on
  macOS vs Linux vs Windows. Out of scope for v1.
- **Community submissions / PR triage automation** — once the cron
  watcher proves the round-trip works.

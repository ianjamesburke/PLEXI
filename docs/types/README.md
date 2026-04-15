# Plexi Type Registry

This directory is the Phase 1 type registry for Plexi typed pipes. Each kind is
declared as a single TOML file. The host reads every `*.toml` under this tree at
startup and builds an in-memory registry keyed by `<namespace>.<name>`.

For the full spec — auto-wire algorithm, patchbay overlay, versioning rules,
wire messages — see [`docs/specs/subsystems/typed-pipes.md`](../specs/typed-pipes.md).

---

## Directory layout

```
docs/types/
  core/          # host-reserved namespace; 6 stable kinds ship with v1
  standard/      # reserved for future cross-vendor standard kinds (empty in v1)
  vendor/        # reserved for app-author-defined kinds (empty in v1)
```

### Tiers

| Tier | Namespace | Who controls it | Review bar |
|---|---|---|---|
| **core** | `core.*` | plexi-core team | Highest. Must be useful for ≥ 3 shipping apps, not expressible as a trivial union on `core.json`, worth teaching forever. Six kinds is the v1 commitment. |
| **standard** | `standard.<topic>.*` | plexi-core + community | Graduated from vendor. Cross-app conventions that have proven themselves in the wild. |
| **vendor** | `vendor.<owner>.*` | App authors | Self-serve. No review required. Lives in the app's own directory or here if the author wants it in the shared registry. |

---

## How to read a type file

Every file follows the same meta-schema:

```toml
name        = "text"           # short kind name; must match filename
namespace   = "core"           # core | standard.<topic> | vendor.<owner>
version     = "1.0.0"          # semver — bump on any schema change
status      = "stable"         # proposed | stable | deprecated | removed
maintainer  = "plexi-core"

description = "..."            # prose: what it means, what it's NOT

[payload_schema]
# For a flat object payload:
fields = [
  { name = "text", type = "string", required = true },
]

# For a discriminated union, use variants instead:
# discriminator = "kind"
# [[payload_schema.variants]]
# kind = "text_range"
# fields = [...]

[examples]
plain = '{ "text": "Hello, world!" }'
```

Field types: `string`, `integer`, `number`, `boolean`, `any`, `object`, or `<type>[]`
(e.g. `string[]`).

**Versioning rule of thumb:** adding optional fields = minor bump; anything
that breaks an existing consumer (rename, remove, type change, new required
field) = major bump. Different majors do not wire together.

---

## Declaring I/O ports in an app manifest

Add an `[app.io]` block to your `manifest.toml`. It is purely additive —
existing manifests without it remain valid and simply declare no channels.

```toml
[app.io]
inputs = [
  { name = "path",      kind = "core.file_path", required = false },
  { name = "goto",      kind = "core.selection", required = false },
]
outputs = [
  { name = "selection", kind = "core.selection" },
  { name = "buffer",    kind = "core.text" },
  { name = "saved",     kind = "core.event" },
]
```

- `name` — per-app channel name; scoped to this app only; ASCII `[a-z0-9_]`
- `kind` — must match a kind in the registry (`namespace.name`); unknown kinds
  fail app load with a clear error
- `required` (inputs only, default `false`) — if `true`, the app refuses to
  start unless at least one matching wire resolves at init time; use sparingly
- `version` (optional) — semver constraint, default `^<current major>`

The host auto-wires channels inside a linked pane group when kinds match and
names are compatible. See the full auto-wire algorithm in the typed-pipes spec.

---

## The v1 core kinds at a glance

| Kind | Payload shape | Typical use |
|---|---|---|
| `core.text` | `{ text: string }` | Prompts, responses, cell contents, captured output |
| `core.json` | `{ value: any }` | Arbitrary structured data — the escape hatch |
| `core.file_path` | `{ path: string }` | Absolute or repo-relative paths |
| `core.selection` | `{ kind, … }` (union) | Text ranges, file lists, list items |
| `core.event` | `{ name, data? }` | Lifecycle signals, clicks, run-complete |
| `core.metric` | `{ name, value, unit?, ts?, tags? }` | Numeric measurements for dashboards |

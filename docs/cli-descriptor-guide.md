# CLI Descriptor Authoring Guide (`--plexi`)

Plexi discovers how to render a UI for any CLI by running `<cli> --plexi` and parsing the JSON it prints. A CLI that responds with a valid descriptor gets a rendered interface automatically — form inputs, live state polling, or a full PGAP app — with no Plexi-specific SDK required.

This guide explains how to write that descriptor.

---

## 1. What `--plexi` is

Plexi resolves a CLI's UI through three tiers, in order:

1. **Native** — run `<cli> --plexi`, parse the JSON it prints to stdout.
2. **Registry** — look up a bundled descriptor in `~/.plexi-<channel>/registry/<cli>/latest.json` (user-managed) or the embedded registry shipped with the binary.
3. **Help crawl** — fall back to parsing `<cli> --help` heuristically. Incomplete, but better than nothing.

Writing a `--plexi` handler (Tier 1) gives you full control over the UI. A registry file (Tier 2) achieves the same result without modifying the CLI — useful for third-party tools you don't own.

---

## 2. Minimal working descriptor

The smallest valid JSON that Plexi accepts:

```json
{
  "plexi_version": "0.1",
  "name": "mytool",
  "version": "1.0.0",
  "commands": []
}
```

`plexi_version` is the descriptor format version, not your CLI's version. Use `"0.1"` for all current descriptors. A major bump (e.g. `"1.0"`) signals a breaking schema change — older Plexi builds will reject it.

---

## 3. Field reference

### Top-level fields

| Field | Type | Required | Description |
|---|---|---|---|
| `plexi_version` | string | yes | Descriptor format version. Use `"0.1"`. |
| `name` | string | yes | Human-friendly CLI name (usually the binary name). |
| `version` | string | yes | Your CLI's own version string. Used as a cache key for registry lookups. |
| `description` | string | no | One-line description rendered in the command palette. |
| `icon` | string | no | Single emoji or glyph shown next to the name. |
| `default_view` | `ui_hint` | no | Initial render mode when no command is selected. See [ui_hint guide](#4-ui_hint-guide). |
| `commands` | array | yes | Top-level subcommands. May be empty. |
| `live_state` | object | no | Tells Plexi how to watch out-of-band state changes. See below. |
| `plexi_app` | string | no | Shell command to spawn as a PGAP process instead of the auto-generated form UI. |
| `capabilities` | array | no | Capability strings granted to the `plexi_app` process (same vocabulary as `manifest.toml`). |

### `commands[]`

Each command in the array (and in nested `commands[]`) accepts:

| Field | Type | Description |
|---|---|---|
| `name` | string | Subcommand token as passed on the CLI. Required. |
| `description` | string | Shown in the command list. |
| `icon` | string | Per-command emoji or glyph. |
| `ui_hint` | `ui_hint` | How Plexi renders this command. Defaults to `form`. |
| `args` | array | Positional arguments, in order. Each entry is an `argSpec`. |
| `flags` | array | Long-form flags (include the leading `--`). Each entry is an `argSpec`. |
| `writes` | array | Filesystem paths this command may write. Used for trust gating. |
| `reads` | array | Filesystem paths this command reads. Hint for capability prompts. |
| `streaming` | boolean | `true` if stdout streams progress over time (vs. one-shot). |
| `output_format` | string | Shape of stdout when `ui_hint = output`. Common: `text`, `json`, `yaml`, `table`. |
| `commands` | array | Nested subcommands — recursive. Models `git remote add`-style hierarchies. |

### `argSpec` (for both `args` and `flags`)

| Field | Type | Description |
|---|---|---|
| `name` | string | Arg name (positional) or long flag (`--foo`). Required. |
| `type` | string | One of: `string`, `int`, `float`, `bool`, `path`, `enum`. Required. |
| `required` | boolean | Whether the arg must be supplied. |
| `default` | any | Default value if not supplied. Type must match `type`. |
| `description` | string | Shown in the form UI. |
| `placeholder` | string | Hint text shown in the input field when empty. |
| `enum_values` | array | Required when `type = "enum"`. The complete set of accepted string values. |
| `min` | number | Inclusive lower bound for `int` / `float`. |
| `max` | number | Inclusive upper bound for `int` / `float`. |

**`path` type:** Plexi may render a file picker. Use it for arguments that accept a filesystem path.

**`enum` type:** `enum_values` is required. Plexi renders a dropdown. Example:

```json
{
  "name": "--format",
  "type": "enum",
  "enum_values": ["json", "yaml", "table"],
  "default": "table"
}
```

### `live_state`

Tells Plexi to watch an external source for state changes while the user is looking at the UI — for example, a manifest your CLI is writing to in the background.

| Field | Type | Description |
|---|---|---|
| `source` | string | `"file"`, `"socket"`, or `"http"`. Required. |
| `path` | string | Filesystem path, socket path, or HTTP URL depending on `source`. Required. |
| `poll_ms` | integer | How often Plexi re-reads the source. Minimum 50 ms. Required. |
| `format` | string | How to parse the payload: `"json"`, `"yaml"`, or `"text"`. Required. |

### `plexi_app` + `capabilities`

When your CLI has a custom UI that goes beyond form inputs, set `plexi_app` to the shell command Plexi should spawn as a PGAP app:

```json
{
  "plexi_app": "mytool-ui --pane",
  "capabilities": ["ai.query", "panes.spawn"]
}
```

Plexi splits `plexi_app` on whitespace — first token is the binary, rest are initial args. `capabilities` uses the same vocabulary as `manifest.toml`. `plexi_app` is the escape hatch for rich custom UI; use it only when the auto-generated form UI is insufficient.

---

## 4. `ui_hint` guide

`ui_hint` tells Plexi how to render a command. Set it at the command level; inherit from `default_view` at the top level.

| Value | What it renders |
|---|---|
| `form` | Input fields + submit button. Runs the command when submitted. |
| `output` | Run button + result pane. Shows stdout after completion. |
| `stream` | Like `output`, but streams stdout in real time. Use with `"streaming": true`. |
| `list` | Browse children. Shows nested subcommands as a navigable list. |
| `tabs` | Grouped subcommands rendered as tabs. |

**Choosing between `output` and `stream`:** use `output` when the command completes quickly and you want to show the result. Use `stream` (with `"streaming": true`) for long-running commands like build pipelines, agent runs, or log tails.

**`list` at the top level:** set `"default_view": "list"` when your CLI has many subcommands and you want the user to browse them rather than see a single form. Each subcommand then declares its own `ui_hint`.

---

## 5. Complete annotated example

The `parallax` video-pipeline CLI, fully annotated:

```json
{
  "plexi_version": "0.1",        // descriptor format version — always "0.1" for now
  "name": "parallax",
  "version": "0.1.0",            // your CLI's version, used for cache keys
  "description": "Video agent pipeline CLI",
  "icon": "🎬",
  "default_view": "list",        // browse commands on first open

  "commands": [
    {
      "name": "run",
      "description": "Kick off a footage_edit run in cwd",
      "icon": "▶",
      "ui_hint": "form",         // show inputs + submit button
      "args": [
        {
          "name": "brief",
          "type": "string",
          "required": true,
          "description": "What you want the agent to create",
          "placeholder": "western cowboy scene, 8 seconds"
        }
      ],
      "flags": [
        {"name": "--test-mode", "type": "bool", "default": false}
      ],
      "writes": [".parallax/"],  // trust gating — Plexi may prompt before allowing writes
      "streaming": true          // stdout streams progress — use stream ui_hint or form+streaming
    },
    {
      "name": "status",
      "description": "Print manifest stats",
      "ui_hint": "output",       // run once, show result
      "args": [],
      "output_format": "yaml"    // hint: Plexi may syntax-highlight the output
    },
    {
      "name": "project",
      "description": "Project management",
      // no ui_hint here — Plexi infers "list" from the nested commands
      "commands": [
        {
          "name": "new",
          "args": [{"name": "name", "type": "string", "required": true}]
        },
        {"name": "list"}
      ]
    }
  ],

  "live_state": {
    "source": "file",
    "path": ".parallax/manifest.yaml",  // watch this file for changes
    "poll_ms": 1000,                     // re-read every second
    "format": "yaml"
  }
}
```

To add `--plexi` support to a CLI, print this JSON to stdout when `--plexi` is the sole argument and exit 0:

```python
import sys, json

DESCRIPTOR = { ... }  # your descriptor dict

if len(sys.argv) == 2 and sys.argv[1] == "--plexi":
    print(json.dumps(DESCRIPTOR))
    sys.exit(0)

# ... rest of CLI
```

---

## 6. Registry fallback (Tier 2)

If you can't modify a CLI to respond to `--plexi`, drop a descriptor at:

```
~/.plexi-<channel>/registry/<cli>/latest.json
```

Where `<channel>` matches the running binary (`alpha`, `beta`, or omitted for stable `~/.plexi/`). Example for the stable build:

```
~/.plexi/registry/mytool/latest.json
```

Plexi checks the user registry before the embedded one, so this always wins. To pin a specific CLI version, name the file `<version>.json` (e.g. `2.40.0.json`) alongside `latest.json`.

Several CLIs ship descriptors in the embedded registry (gh, cargo, npm, git, docker, kubectl, uv, parallax). These live in `registry/` at the PLEXI repo root and are baked in at build time.

---

## 7. Verification

After writing a descriptor or dropping a registry file, confirm Plexi resolves and parses it correctly:

```
plexi descriptor probe <cli>
```

Example output for parallax (via registry):

```
🎬 parallax v0.1.0  (descriptor 0.1)  (via registry)
  Video agent pipeline CLI
commands: 3
  - run [form] — Kick off a footage_edit run in cwd
  - status [output] — Print manifest stats
  - project (+2 subcommands) — Project management
live_state: File .parallax/manifest.yaml (poll 1000 ms, Yaml)
```

If `probe` fails with "no descriptor found", check:
1. Does `<cli> --plexi` print valid JSON to stdout and exit 0?
2. Is the registry file at the right path with the right filename (`latest.json`)?
3. Does the JSON pass schema validation? Run it through `cat <file> | jq .` to check syntax, then compare against `schemas/plexi-descriptor-schema.json`.

The schema file lives at `schemas/plexi-descriptor-schema.json` in the PLEXI repo and has inline field descriptions for every property.

#!/usr/bin/env python3
"""Generate website/src/content/docs/pgap.md from sdk/protocol/pgap.schema.json.

Reads the JSON Schema produced by `gen_schema` and emits a capability reference
page with message-type tables: type discriminant, description, required fields,
and capability gate.

Run via: just gen-capability-docs
"""
from __future__ import annotations

import json
import re
import sys
import textwrap
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = REPO_ROOT / "sdk" / "protocol" / "pgap.schema.json"
OUTPUT_PATH = REPO_ROOT / "website" / "src" / "content" / "docs" / "pgap.md"

CAP_RE = re.compile(r"(?:[Rr]equires?|[Gg]ated on)\s*`([^`]+)`(?:\s+capability)?")

FRONTMATTER = """\
---
title: PGAP
description: The Plexi Generic API Protocol — how plugins communicate with the host.
order: 3
---
"""

INTRO = """\
PGAP (Plexi Generic API Protocol) describes messages between plugins and the
Plexi host. The Python adapter bridges these JSON messages into host services.
Rust WASM components use the typed contract in `wit/plexi.wit`.

## How It Works

Opening a view starts its plugin runtime. The plugin returns a UI tree;
the host paints it and delivers input events for the containing pane.

```
Host ──render request──► Plugin
Host ◄──UI tree───────── Plugin
Host ──input event──────► Plugin
```

The host owns the render loop. When a view needs rendering, the host asks its runtime
for a fresh set of draw commands and composites them into the UI.

Every message is a JSON object with a `type` discriminant field. The schema
version is `{version}` — see `sdk/protocol/pgap.schema.json` for the full
machine-readable definition.

## Capabilities

Every plugin declares its capabilities in `manifest.toml`. The host enforces
these at runtime:

```toml
[plugin.capabilities]
capabilities = ["secrets.get", "net.http"]
```

Capabilities gate host APIs. Python uses the CPython WASI adapter; Rust
components use typed WIT imports. These are distinct runtime paths with their
own admission and permission checks. The **Cap** column shows the declared
capability associated with each request; other scope and broker checks can
still apply. See [the runtime contract](https://github.com/IanBurke/PLEXI/blob/alpha/docs/wasm-runtime.md).

## The Python SDK

The Python SDK uses module-level `init`, `update`, and `view` functions.
`view()` returns a component tree; `PluginHarness` exercises the lifecycle.
See the [SDK reference](/docs/sdk) for the full API.
"""


def _fmt_type(schema: object) -> str:
    """Return a compact type string for a property schema."""
    if not isinstance(schema, dict):
        # JSON Schema allows `true` / `false` as schemas.
        return "any" if schema else "never"
    if "$ref" in schema:
        return schema["$ref"].split("/")[-1]
    t = schema.get("type")
    if isinstance(t, list):
        types = [x for x in t if x != "null"]
        base = types[0] if types else "any"
        return f"{base}?" if "null" in t else base
    if t == "array":
        items = schema.get("items", {})
        return f"{_fmt_type(items)}[]"
    if t == "object":
        return "object"
    if "const" in schema:
        return f'"{schema["const"]}"'
    if "oneOf" in schema or "anyOf" in schema:
        return "variant"
    return t or "any"


def _extract_fields(item: dict) -> list[tuple[str, str, str]]:
    """Return [(field_name, type_str, required)] for an item's properties.

    Excludes the `type` discriminant — it's shown in the Type column already.
    """
    props = item.get("properties", {})
    required = set(item.get("required", []))
    rows: list[tuple[str, str, str]] = []
    for name, schema in props.items():
        if name == "type":
            continue
        type_str = _fmt_type(schema)
        req = "yes" if name in required else "no"
        rows.append((name, type_str, req))
    return rows


def _extract_capability(desc: str) -> str:
    m = CAP_RE.search(desc)
    return m.group(1) if m else ""


def _short_desc(desc: str) -> str:
    """First sentence of a description, stripped of newlines."""
    first = desc.split("\n\n")[0].replace("\n", " ").strip()
    # Trim at 120 chars to keep tables readable.
    if len(first) > 120:
        first = first[:117] + "..."
    return first


def _render_message_table(items: list[dict]) -> list[str]:
    """Render a markdown table for a list of oneOf schema items."""
    lines: list[str] = []

    for item in items:
        props = item.get("properties", {})
        ty = props.get("type", {}).get("const", "")
        if not ty:
            continue
        desc = item.get("description", "")
        cap = _extract_capability(desc)
        short = _short_desc(desc)
        fields = _extract_fields(item)

        cap_badge = f"`{cap}`" if cap else ""

        lines.append(f"### `{ty}`")
        lines.append("")
        if short:
            lines.append(short)
            lines.append("")
        if cap_badge:
            lines.append(f"**Capability:** {cap_badge}")
            lines.append("")
        if fields:
            lines.append("| Field | Type | Required |")
            lines.append("|-------|------|----------|")
            for fname, ftype, freq in fields:
                lines.append(f"| `{fname}` | `{ftype}` | {freq} |")
            lines.append("")
        else:
            lines.append("*No additional fields.*")
            lines.append("")

    return lines


def generate(schema_path: Path = SCHEMA_PATH) -> str:
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    version = schema.get("version", "unknown")
    defs = schema.get("definitions", {})

    out: list[str] = []
    out.append(FRONTMATTER.rstrip())
    out.append("<!-- Generated by tools/gen_capability_docs.py — do not edit by hand. -->")
    out.append("<!-- Run `just gen-capability-docs` to regenerate. -->")
    out.append("")
    out.append(textwrap.dedent(INTRO.format(version=version)).strip())
    out.append("")

    sections: list[tuple[str, str, str]] = [
        ("HostRequest",     "App → Host Requests",  "Messages the app sends to request host services."),
        ("PlexiEvent",     "Host → App Events",    "Messages the host sends to the app."),
        ("RenderCommand",  "Render Commands",       "Draw primitives the app emits each frame."),
        ("ControlCommand", "Control Commands",      "Inline commands processed outside the render loop."),
    ]

    for def_key, title, blurb in sections:
        defn = defs.get(def_key)
        if defn is None:
            continue
        items = defn.get("oneOf", [])
        out.append(f"## {title}")
        out.append("")
        defn_desc = defn.get("description", "")
        if defn_desc:
            out.append(_short_desc(defn_desc))
            out.append("")
        out.append(blurb)
        out.append("")
        out.extend(_render_message_table(items))

    text = "\n".join(out).rstrip() + "\n"
    return text


def main() -> None:
    if "--stdout" in sys.argv:
        sys.stdout.write(generate())
        return
    content = generate()
    OUTPUT_PATH.write_text(content, encoding="utf-8")
    print(f"Written: {OUTPUT_PATH.relative_to(REPO_ROOT)}")


if __name__ == "__main__":
    main()

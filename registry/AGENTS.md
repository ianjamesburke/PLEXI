# registry — Agent Contract

**Read before editing anything under registry/:** this file, plus the root AGENTS.md.

## Scope

Embedded CLI descriptor registry. JSON descriptors here are baked into the binary at build time and serve as Tier 2 fallbacks for `plexi app open --cli`.

## Reference

- [CLI_DESCRIPTOR_GUIDE.md](CLI_DESCRIPTOR_GUIDE.md) — full descriptor authoring guide: field reference, `ui_hint`, `live_state`, `plexi_app`, registry fallback, verification.

## Rules

- Descriptors must pass `plexi descriptor probe <cli>` after any edit.
- Schema reference: `schemas/plexi-descriptor-schema.json`.
- User-managed registry files live at `~/.plexi-<channel>/registry/<cli>/latest.json`, not here.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.

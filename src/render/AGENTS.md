# src/render — Agent Contract

**Read before editing anything under src/render/:** this file, plus the root AGENTS.md.

## Scope

Host-side rendering: CLI renderer app, app render pipeline, draw command dispatch.

## Reference

- [CLI_APP_CONTRACT.md](CLI_APP_CONTRACT.md) — CLI-backed app runtime contract: launch sequence, ready/run/reload lifecycle, descriptor caching, permissions, logging, known gaps.

## Traps

- **CLI-backed apps are builtins, not PGAP subprocesses.** No manifest, no capability checks. The renderer runs on the host UI thread with host privileges.
- **The descriptor is frozen at open time.** `CliRendererApp::new()` reads the temp file once. No watch/reload. Close and re-open to refresh.
- **`serialize_state()` returns `None`.** CLI-backed panes do not survive layout restore.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.

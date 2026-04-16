Always confirm best practices by researching the docs.

## North Star

- [`STATE_OF_PLEXI.md`](STATE_OF_PLEXI.md) — current architecture, reality check, critical path. Read first.
- [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md) — the v3 spec. Single source of truth for the protocol, pane ADT, secrets invariant, media, Plexi IQ, example apps.
- [`docs/specs/README.md`](docs/specs/README.md) — spec index.

If a doc outside these contradicts them, the doc is wrong. Fix or delete it.

## Terminology

**PGAP** — Plexi Generic App Protocol. Newline-delimited JSON over stdin/stdout. `PlexiEvent` flows host→app, `DrawCommand` flows app→host. Binary data (audio PCM, video frames, raw bytes) travels on typed pipes, not stdio. PGAP is the isolation boundary — no shared memory, no inherited FDs.

## Branches

- `main` — stable releases.
- `beta` — staging.
- `alpha` — frozen v2.x tree, tagged `v2-last`, retired. Do not land new work here.
- `v3` — active development for the v3.0 clean cut. All feature branches cut from `v3`, worked in `.claude/worktrees/`, merged back via PR.

Feature branch naming: `feature/<issue-number>-short-description`. Sub-agent workflow: `isolation: "worktree"` off `v3`, PR back to `v3`. Never push directly to long-lived branches.

## GitHub Issue Labels

Every issue gets one **type**, one **priority**, one **version**.

- **type:** `bug` | `enhancement` | `idea`
- **priority:** `P1` (shipping blocker) | `P2` | `P3` | `P4`
- **version:** `v3.0` | `v3.1+` | `future`
- **status** (optional): `in-progress` | `ready` | `blocked`

## App Installation Paths

Build-specific, resolved at runtime by binary name:

| Build | Apps directory |
|---|---|
| Alpha (frozen) | `~/.plexi-alpha/apps/` |
| Beta | `~/.plexi-beta/apps/` |
| Stable | `~/.plexi/apps/` |
| v3 dev build | `~/.plexi-v3/apps/` |

Each app is a subdirectory with `manifest.toml` and an executable entry point. Installing to the wrong directory silently does nothing.

## Build & Install

`just install` runs `cargo bundle --release`, copies the `.app` to `/Applications`, extracts the binary to `/usr/local/bin/plexi`, then runs `lsregister -f <bundle>` and `pbs -update` to refresh macOS Services.

**After every completed code change, install for the active branch** before reporting the task complete:
- `v3` → `just install-v3`
- `main` → `just install`

## Logging

Build-specific log file:
- v3: `~/.plexi-v3/plexi.log`
- Stable: `~/.plexi/plexi.log`

Rotates to `plexi.log.1` at 10 MB. Level set in `config.toml` (`error | warn | info | debug`). Third-party crates clamped to `warn`.

App logs forward into the host log tagged `app::<app_id>`. Python SDK: `ctx.info/warn/error/debug(...)` inside a frame; `emit.info(...)` outside. App stderr forwards as `warn`-level `app::<app_id>` entries.

**When debugging, check the log file first.**

## Configuration Philosophy

Required fields have no defaults — fail fast with a clear error. Optional fields are clearly marked. Never paper over ambiguity with invisible magic. Prefer a verbose generated config with all options visible over a sparse one with hidden behavior.

## Python Tooling

Use `uv` for all Python projects. `pyproject.toml` with `requires-python = ">=3.11"`, `uv sync`, `uv run`. Bootstrap with `curl -LsSf https://astral.sh/uv/install.sh | sh` if absent. Never write manual venv creation loops.

## Error Handling

Try-catch on all I/O, network, external API calls, and anything that can reasonably fail. Every catch logs where + what failed with enough context to diagnose. Never swallow errors silently. If a failure can't be meaningfully recovered from, propagate or re-throw.

## Lessons Carried Into v3

- **Python version in GUI app bundles:** macOS GUI bundles do NOT inherit shell PATH. `#!/usr/bin/env python3` → Apple's frozen `/usr/bin/python3` 3.9.6. Always add `from __future__ import annotations` as the first line of every app Python file so `str | None` is safe on 3.7+.
- **Install doesn't chmod:** `just install-*` syncs files but doesn't set executable bits. Run `chmod +x ~/.plexi-*/apps/*/*.py` after install, or fix the recipe.
- **Coupled state:** When adding state that derives from or shadows existing state, grep every mutation site of the original and update each one.
- **Fallback chain audit:** When a value looks correct on the surface but behavior is stale, enumerate every fallback source in priority order (cookies, env vars, caches, defaults). Fix the chain, not the surface.
- **Model ID verification:** Never guess versioned model IDs. Use only confirmed-current family IDs. A 400/404 surfaces only at call time.

## General Rules

- Before SSH/networking setup, ask if machines are on the same LAN or remote. Before any multi-step infra task, clarify topology first.
- When the user reports a bug, fix what they asked for first. Don't pivot to QA, refactoring, or tangential improvements until the primary request is resolved.
- When the user provides multiple distinct ideas, file them separately. Don't combine unrelated concepts.

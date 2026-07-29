---
name: drive-host
description: "Live headed end-to-end loop for validating a PR (or any channel) build against a real running Plexi host. Boots the host with `plexi host start`, drives it with pane/events commands, observes ground truth from pane state/capture and the channel log, then tears it down. Use when a change can only be verified in the installed binary (PTY/terminal flows, menus, dock, real multi-pane behavior) — not for anything a headless harness can cover."
risk: medium
source: local
date_added: "2026-07-02"
---

# Drive Host — Live Headed E2E Loop

Validate a change against the real, running host binary. `plexi host start` (stint 0342) turned the old hand-rolled nohup/env/socket dance into one command; this skill is the loop built on top of it.

## When to reach for this layer

Three testing layers exist. Pick the cheapest one that can observe the behavior — only escalate to this skill when nothing headless can see it.

| Layer | What it exercises | Use when | Reference |
|---|---|---|---|
| HostHarness / TOML scenes | HostModel logic + egui render, fully headless | return values, effects, overlay/widget layout, canvas clicks (`HostHarness::inject_click`, stint 0398), anything a scene can assert | `src/testing/TESTING.md` |
| AppHarness | one PGAP app subprocess over the protocol, headless PNG | Python app layout, key handling, pixel/no-overlap assertions | `src/testing/TESTING.md` |
| **drive-host (this skill)** | the installed `.app` binary, headed, real IPC | PTY/terminal interaction, keyboard-capture flows, menus/dock/file-associations, real multi-pane orchestration — behavior only observable in the running bundle | here |

If a HostHarness test or committed scene can reproduce it, write that instead — it is deterministic and runs in CI. This skill is the escape hatch for the rest.

When a committed TOML scene already describes the flow, prefer `just scene-live <scene> <channel>` over spelling out this loop manually. It uses the same scene schema and report shape as the headless backend while owning readiness, bounded eventual assertions, and teardown.

## The loop

All four steps run against one channel. For a PR build the channel is `pr-<N>` and the binary is `plexi-pr-<N>`; substitute your channel throughout. Current channel-suffixed binaries resolve commands from their own config dir (`~/.plexi-<channel>/notify.sock`). Keep the explicit socket and channel environment below so the loop also works with older installed binaries.

### 1. Boot

Seed a known layout and block until the host answers a socket round-trip:

```bash
plexi-pr-<N> host start --pane 'cwd=/tmp,cmd=htop' --pane 'cwd=/tmp'
```

`--pane 'cwd=<dir>[,cmd=<command>][,tab|window]'` is repeatable; `--layout <file.toml>` seeds the same `[[pane]]` specs from a committed fixture (combine both — they append). `--timeout-secs <n>` overrides the 15s readiness wait. `host start` errors loudly if a host for this channel is already up (prints pid + socket) — `host stop` first if so. Full flag surface: `.stint/tasks/0342-host-start-declarative-boot.md`.

Confirm readiness before driving:

```bash
plexi-pr-<N> host status --json
```

`ready: true` with the expected `pane_count` means boot succeeded. Never start sending pane commands off an assumption that boot finished.

### 2. Drive

Read the seeded panes, then act on them by explicit id:

```bash
plexi-pr-<N> pane list
plexi-pr-<N> pane send <pane-id> "git status\n"
plexi-pr-<N> pane key <pane-id> enter
plexi-pr-<N> pane click <pane-id> <x> <y> [--button left]
plexi-pr-<N> pane new "npm run dev" -n dev
```

`pane click` (stint 0398) injects a synthetic pointer click at PANE-PIXEL coordinates (origin at the pane's top-left) into an app pane — this is the sanctioned way to drive canvas interaction (games, grids, any `Canvas` node). It is delivered through the pane's real `canvas_transform` inversion (the same math a physical click would hit), not a shortcut. **Never reach for OS-level automation (`osascript`, `cliclick`) to simulate a click** — it trips the macOS Accessibility permission prompt this skill is meant to avoid, and `pane click` covers the same ground through a sanctioned host primitive. Only drag/hover/scroll remain unsupported (press/click only, non-scope of stint 0398).

Every drive command needs the env discipline below. Always target panes by the id from `pane list` — never rely on "current pane" defaults, which read stale `PLEXI_*` vars from the pane you launched this skill in.

**Env discipline.** Current channel-suffixed binaries reject a socket inherited from another channel. Keep pointing every drive command at the PR host's socket for compatibility with older binaries, and strip inherited pane/context identity because those values still describe the launching pane:

```bash
env -u PLEXI_PANE_ID -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_ROOT -u PLEXI_RUNNING \
  PLEXI_SOCKET=$HOME/.plexi-pr-<N>/notify.sock PLEXI_CHANNEL=pr-<N> \
  plexi-pr-<N> pane list
```

To watch an app's event streams, subscribe (NDJSON, one object per line) with the same env prefix:

```bash
env -u PLEXI_PANE_ID ... PLEXI_SOCKET=$HOME/.plexi-pr-<N>/notify.sock PLEXI_CHANNEL=pr-<N> \
  plexi-pr-<N> events subscribe <app_id> <stream>
```

### 3. Observe

Ground truth comes from the host, never from what you expect to have happened:

```bash
plexi-pr-<N> pane state <pane-id>     # app panes: normalized semantic tree; terminals: status
CAPTURE_META=$(mktemp /tmp/plexi-capture-meta.XXXXXX)
plexi-pr-<N> pane capture <pane-id> --lines 80 --plain 2>"$CAPTURE_META"
CAPTURE_CURSOR=$(sed -n 's/^cursor=//p' "$CAPTURE_META")
# Later: print only output/redraw deltas and advance the cursor again.
NEXT_CAPTURE_META=$(mktemp /tmp/plexi-capture-meta.XXXXXX)
plexi-pr-<N> pane capture <pane-id> --plain --from-cursor "$CAPTURE_CURSOR" \
  2>"$NEXT_CAPTURE_META"
tail -100 ~/.plexi-pr-<N>/plexi.log
```

`pane state` exposes the last committed normalized semantic tree for process, native builtin, and WASM app panes — assert against `semantic.nodes`, not against a screenshot you eyeballed. Process panes also retain their compatible `frame` field. `pane capture --plain` keeps stdout byte-clean for terminal assertions and writes `cursor=<N>` to stderr; feed that cursor to `--from-cursor` for the next changed-row delta. The channel log (`~/.plexi-pr-<N>/plexi.log`) is the source for anything the drive commands can't surface — every feature ships an `info` trace, so grep it for the behavior you seeded.

### 4. Teardown

```bash
plexi-pr-<N> host stop
plexi-pr-<N> host status
```

`host stop` sends a clean shutdown request, falling back to SIGTERM. Confirm the process is gone with `host status` (expect "not running") before declaring done — a lingering host blocks the next `host start` for that channel.

## Rules

- Escalate to this layer only when a HostHarness test or committed scene genuinely cannot observe the behavior. Deterministic headless coverage wins.
- Verify `host status` readiness before driving, and process-gone after `host stop`. Never chain a step on an assumed prior step.
- Every drive command (`pane *`, `events subscribe`) carries the env prefix: explicit `PLEXI_SOCKET` for the target channel, explicit `PLEXI_CHANNEL`, and stripped inherited `PLEXI_PANE_ID` / `PLEXI_CONTEXT_ID` / `PLEXI_CONTEXT_ROOT` / `PLEXI_RUNNING`.
- Target panes by explicit id from `pane list` — never "current pane" defaults, which read the launching pane's stale identity.
- Observe from `pane state` / `pane capture` / the channel log, never from assumption.
- One host per channel. `host stop` before re-booting; leave no orphaned host behind.
- The flag surface lives in stint 0342 and `--help`; do not restate it — link it.

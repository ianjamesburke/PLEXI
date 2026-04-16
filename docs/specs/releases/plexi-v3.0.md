# Plexi v3.0 — Clean Cut

**Status:** Draft
**Date:** 2026-04-16
**Supersedes:** all v2.0 / v2.1 / v2.2 / v2.3 drafts where noted in §12

> v3.0 is a deliberate break. It throws out recursion, OpenIntent-as-spec'd, and most of the example app library. It keeps the good v2.0 ideas (PGAP, capability broker, event bus, runs, notifications, secrets, Plexi IQ) and rebuilds them cleanly, adds host-owned media primitives with a binary side channel via typed pipes, and ships with five focused example apps. Breaking changes are expected and welcome. The goal is stable.

---

## 1. Goals & Non-Goals

### Goals
- One clean PGAP v3 protocol, fully testable via JSON replay + mock devices.
- Pane ADT (`Terminal` / `App` / `Agent`) wired correctly from the first commit.
- Directory-scoped secrets as a **hard invariant**: no leaks across workspace boundaries without brokered prompt.
- Host-owned audio + video subsystem with a binary side channel (typed pipes) for raw PCM / frames.
- Plexi IQ wired into `Pane::Agent` from day one — no dead-code modules.
- Five example apps, no more: `snake`, `wikipedia`, `todo`, `audio-recorder`, `video-player`.

### Non-Goals (explicit)
- **No fractal PGAP.** No recursion, no `.plexi` boundaries, no depth tree, no embedded mode, no portals, no `Pane::Embedded`.
- **No live-DSP plugin surface.** Real-time per-sample synthesis inside an external app process is out of scope. Reserved in §11 as a future v4+ surface. v3 supports buffered playback, recording, sample triggering, and offline synthesis.
- **No OpenIntent as previously spec'd.** Folded into a simpler `Init` payload.
- **No `Pane::Embedded`, no `DepthTransition`, no `TreeStatus`, no `plexi --embedded` mode.**

---

## 2. Pane ADT

```rust
enum Pane {
    Terminal(TerminalPane), // PTY + vt100
    App(AppPane),           // external PGAP child process
    Agent(AgentPane),       // Plexi IQ instance
}
```

No other variants. Adding a variant requires a spec amendment.

---

## 3. PGAP v3 — The Protocol

Newline-delimited JSON over a child process's stdin/stdout. Binary frames travel on **typed pipes**, not stdio (§7).

### 3.1 Init handshake
Host spawns the app and sends exactly one `Init`:

```json
{"type":"Init","protocol":"pgap/3","app_id":"audio-recorder","workspace_root":"/Users/x/projects/foo","capabilities":["audio.record","fs.read"],"feature_flags":["media_v1","pane_groups_v1"]}
```

App replies with exactly one `Ready`:

```json
{"type":"Ready","sdk":"plexi-sdk-py/0.4.0","features_used":["media_v1"]}
```

Version negotiation: app MUST refuse `protocol` values it doesn't understand. Feature flags are additive; unknown flags are ignored.

### 3.2 PlexiEvent (host → app)
- `Render { frame_id, rect }` — request a frame.
- `Input { key | click | scroll | text }` — user input.
- `CapabilityDecision { request_id, granted: bool }` — response to a runtime prompt.
- `SecretValue { key, value | denied }` — secret broker response.
- `RunUpdate { run_id, status, payload }` — run lifecycle.
- `PipeMessage { pipe_id, json_payload }` — typed pipe (JSON mode).
- `PathChanged { cwd }` — pane group broadcast (§8).
- `Suspend` / `Resume` — lifecycle.
- `Shutdown` — terminal; app must exit cleanly.

### 3.3 DrawCommand (app → host)
Frame-scoped:
- `Rect`, `Text`, `Line`, `Image` — visual primitives.
- `VideoPlayer { source, rect, state: Play|Pause|Seek(ms) }` — host owns decoder.
- `AudioMeter { rect, pipe_id }` — bound to an audio pipe for level display.
- `FrameDone { frame_id }` — required terminator.

Out-of-frame:
- `Log { level, message }`
- `CapabilityRequest { request_id, capability }`
- `SecretGet { key }` — scoped to `workspace_root` (§5).
- `RunGet { intent, payload }` / `RunComplete { run_id, result }`
- `Notify { level, title, body, actions? }`
- `AudioPlay { source|pipe_id, volume, state }` — host owns audio device.
- `AudioCapture { pipe_id, sample_rate?, buffer_size? }` — host streams PCM to pipe.
- `PipeOpen { pipe_id, mode: Json|Binary, direction: In|Out|Duplex }`
- `PipeSend { pipe_id, json_payload }` (for JSON-mode pipes)
- `StatusSummary { text }` — shown in parent chrome.

Binary payloads (raw PCM, video frames, arbitrary bytes) travel on typed pipes, never on stdio. See §7.

---

## 4. Capability Broker

Every capability an app uses must be either (a) declared in `manifest.toml`, or (b) runtime-prompted via `CapabilityRequest`. Decisions persist to `permissions.json` keyed by `(app_id, workspace_root, capability)`.

Defined capabilities (v3):
- `fs.read` / `fs.write` — scoped to `workspace_root`.
- `net.http` — outbound HTTP(S).
- `secrets.get` — required to call `SecretGet`.
- `audio.record` / `audio.playback` — access to host audio device.
- `video.playback` — access to host video decoder.
- `pipe.open` — create typed pipes.
- `spawn.app` — launch another app in a new pane.

Undeclared capabilities block at first use and surface a modal prompt.

---

## 5. Directory-Scoped Secrets — Hard Invariant

**Invariant:** A secret fetched by app `A` at `workspace_root = /foo` cannot be read by any app at any other `workspace_root` without a new brokered user prompt. No exceptions. Not even child directories. Not even the same app launched from a sibling.

### Mechanism
- Secrets stored in Keychain with key `plexi/{workspace_root}/{secret_key}`.
- `SecretGet { key }` from app resolves against app's declared `workspace_root` from `Init` only.
- If the secret doesn't exist at that scope, broker emits a UI prompt; user enters or denies.
- User-granted secret is written to Keychain under the *exact* `workspace_root` — never a parent path, never a shared "global" scope.
- Sibling/child launches get denied-by-default and must prompt.

### What this guarantees
- Cloning a project to a sibling directory does not share secrets.
- A malicious app cannot escalate by re-launching itself with a parent `workspace_root` — the host validates `workspace_root` against the pane's actual CWD at spawn.

### What this does not guarantee
- Filesystem access to the Keychain itself (that's macOS's problem, not Plexi's).
- Cross-machine sync (future Plexi Teams work).

---

## 6. Event Bus, Runs, Notifications

### 6.1 Event bus
Single append-only `events.jsonl` at `~/.plexi/events.jsonl`. One `HostEvent` enum:
- `AppSpawned` / `AppClosed`
- `PermissionDecision`
- `SecretPrompted` / `SecretDenied`
- `RunStarted` / `RunUpdated` / `RunCompleted`
- `NotificationPosted` / `NotificationActionInvoked`
- `AgentTurn { pane_id, tokens_in, tokens_out, cost_cents }`
- `PipeOpened` / `PipeClosed`

### 6.2 Runs
`RunGet { intent, payload }` creates a run; host surfaces it in the Run palette (Cmd+R). Runs can be `Pending | Running | BlockedOnUser | Completed | Failed`. `BlockedOnUser` surfaces an inline prompt.

### 6.3 Notifications
`Notify { title, body, level, actions }`. Actions are structured: `{ label, action_type: "resume_run" | "open_intent" | "run_command", payload }`. **All three action types MUST be wired from day one.** No TODOs in the dispatch handler.

---

## 7. Typed Pipes — With Binary Side Channel

Typed pipes are named side channels between apps (or app↔host) that travel **out of band** from stdio. Two modes:

### 7.1 JSON mode
NDJSON messages, routed via `PipeMessage` / `PipeSend` on the main PGAP wire. Good for control/metadata.

### 7.2 Binary mode (the side channel)
Raw bytes over a dedicated OS pipe (unix domain socket on macOS). Host allocates the socket pair on `PipeOpen`, hands one end to the app, keeps the other. Frames are length-prefixed (`u32 BE length || payload`). No JSON overhead.

Binary pipes are the transport for:
- **Audio PCM** — `AudioCapture` opens a binary pipe; host streams `f32` interleaved samples at negotiated sample rate and buffer size. App reads them for display, analysis, or write-to-disk.
- **Video frames** — raw decoded frames (only when app requests them; normally host draws direct to screen via `VideoPlayer`).
- **Arbitrary media buffers** — photo pixel data, waveform arrays, anything that isn't safe to base64 through JSON.

### 7.3 Audio pipe contract
`AudioCapture { pipe_id, sample_rate: 48000, buffer_size: 512 }` → host opens binary pipe → host's audio thread captures from device into a lock-free ring → drain thread writes to pipe → app reads. **Audio thread never blocks on the pipe.** If app can't keep up, host drops frames and emits a `PipeOverrun` event.

### 7.4 Realtime safety note
The host's audio thread is realtime-safe (no alloc, no locks, no syscalls that block). Everything crossing the PGAP boundary is not. v3 does not support in-process live-DSP. See §11.

---

## 8. Pane Groups & Linked Panes

Panes can opt into a **pane group** at spawn via `manifest.toml`:

```toml
[launch]
join_group = "cwd"
layout_hint = { side = "right", split = 0.5 }
```

Apps in the same group receive each other's `PathChanged { cwd }` broadcasts routed by the host. A terminal app responds by `cd`-ing; a file explorer responds by refreshing. Apps do not know about each other — only the group name.

This is the entire "linked panes" surface. No protocol primitive for "these two panes are linked." No visual lines. If a terminal and file explorer end up side by side with synced CWD, that's the group doing its job.

---

## 9. Plexi IQ — Wired From Commit #1

- `Pane::Agent` owns an `IqInstance`.
- `IqInstance` holds a `Backend` (trait object: `ClaudeCli | AnthropicApi | Mock`).
- Turn loop is synchronous with `stream_to_channel`; UI thread never blocks on async runtime.
- Every turn appends to `ledger.jsonl`: `{ timestamp, pane_id, tokens_in, tokens_out, cost_cents, model }`.
- `agent_mode.rs` and `agent_llm.rs` from v2.x are **deleted**. One path in, one path out.
- No `#[allow(dead_code)]` on the IQ module. Ever.

---

## 10. Mock Devices & Test Harness

### 10.1 Device traits
- `AudioDevice` — prod impl wraps CoreAudio; mock impl reads input from a WAV file and writes output to a WAV file.
- `VideoDecoder` — prod impl wraps AVFoundation; mock impl emits a procedurally generated frame sequence or reads from a fixture.
- `MidiDevice` (reserved, not used in v3 example apps).

### 10.2 Env-driven mocking
```sh
PLEXI_AUDIO=mock://fixtures/in.wav,/tmp/out.wav plexi
PLEXI_VIDEO=mock://fixtures/test.mp4 plexi
```
Entire media subsystem becomes deterministic and headless.

### 10.3 Protocol test harness
Existing pattern: replay `PlexiEvent` JSON into an app, assert on emitted `DrawCommand` JSON. Combined with mock devices, CI runs the full example app suite end-to-end with zero real hardware.

---

## 11. Example Apps

Exactly five. No more.

| App | Purpose | Language | Capabilities |
|---|---|---|---|
| `snake` | Keep it for joy. Proves input + draw primitives. | Rust | none |
| `wikipedia` | Proves `net.http` + text rendering. | Python | `net.http` |
| `todo` | Proves `fs.read` + `fs.write` + persistence. | Python | `fs.read`, `fs.write` |
| `audio-recorder` | Proves `audio.record` + binary pipe + mock device. Record from mic (or mock) → WAV file. Live mic-level meter. Start/stop. Nothing else. | Python | `audio.record`, `fs.write` |
| `video-player` | Proves `video.playback` + `VideoPlayer` draw command. Load local mp4, play/pause/seek. Nothing else. | Python | `video.playback`, `fs.read` |

All other v2.x example apps are **not carried forward**. They can return post-v3 if genuinely useful.

### 11.1 First-party apps

First-party apps ship bundled with Plexi but are **implemented entirely on top of the same primitives third-party apps use**. They receive no special host access.

**Invariant:** No first-party app may access host internals not available to third-party apps. If a first-party app needs a capability, that capability must be a declared PGAP capability.

v3 first-party apps:

| App | Purpose | Capabilities | Notes |
|---|---|---|---|
| `quick-note` | Scratchpad / backlog capture. Opens via palette (Cmd+Shift+N), reads/writes markdown files in a declared notes directory, posts notifications when new notes land. Replaces the v2.x host-internal backlog scanner. | `fs.read`, `fs.write`, `notify` | Backlog scanning is removed from `notification_palette.rs` and moved into this app. |

The v2.x backlog-scanning code in `notification_palette.rs` is deleted. The palette remains a host-level UI primitive; notification *content* comes from apps via `Notify`.

---

## 12. Removed / Deferred

### Removed from v2.x
- Fractal PGAP: recursion, `.plexi` boundaries, depth tree, `plexi --embedded`, `DepthTransition`, `TreeStatus`, portals, `Pane::Embedded`.
- OpenIntent as spec'd in v2.0 — folded into `Init`.
- `agent_llm.rs` path — replaced by Plexi IQ.
- All example apps except the five in §11.
- `experiments/v2-*` branches — cherry-pick valuable pieces onto v3, then delete.

### Reserved for future releases (not v3)
- **In-process DSP plugin surface** (v4+) — required for live synthesis, real-time effects, <5 ms analog-modeling instruments. Needs shared memory ring buffers and a plugin sandbox. Conceptually VST/AU/CLAP-shaped.
- **MIDI subsystem** — trivial protocol-wise (capabilities + events + optional typed pipe for SysEx). Skipped in v3 to keep example app count at five.
- **Collaboration / sync** — SpacetimeDB-shaped future work.
- **Spatial canvas / WASM target** — parked v2.3 ideas.
- **Rich text / clip regions / IME / multiline input** — parked v2.2 ideas. Re-evaluate after v3 ships.

---

## 13. Migration Path

v3 is a clean break. There is no in-place upgrade from v2.x on alpha. Plan:

1. Freeze `alpha` as-is, tag `v2-last`.
2. Open `v3` long-lived branch from `main` (last stable: v1.1.x).
3. Port the solid pieces forward: Pane ADT (finished, not PR1-of-3), capability broker, event bus skeleton, runs, notifications, secret broker (with directory-scoped invariant), typed pipes, Plexi IQ (wired).
4. Build media subsystem (audio device trait, video decoder trait, binary pipes, mock impls).
5. Build the five example apps.
6. CI gate: full protocol test harness green with mocked devices.
7. `v3` → `beta` → `main`. Tag `v3.0.0`.

Breaking changes expected: manifest format changes, PGAP version bump, capability list changes, example app removal. All documented in `CHANGELOG.md` at release time.

---

## 14. Open Questions

- **Pane group naming** — free-form strings or a finite vocabulary? Lean toward free-form with a few reserved names (`cwd`, `selection`).
- **Audio sample rate negotiation** — app requests, host can refuse or downsample? Default: host always streams at device rate, app handles resampling if it cares.
- **Binary pipe backpressure policy** — drop oldest, drop newest, or block? For audio capture, drop-oldest with overrun event. For file transfer, block.
- **Keychain prompt UX at first `SecretGet`** — inline modal vs palette entry? Lean toward inline modal, matching capability prompt UX.

These resolve during implementation, not before.

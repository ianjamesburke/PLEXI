# Plexi Security Model

> **Short version:** Plexi v1 apps are native Python processes. The trust boundary
> is **consent + audit**, not process isolation. Install only apps you trust.

---

## Current Model: Consent + Audit

Plexi v1 uses a **capability-gated, consent-based** security model. Apps are native
Python subprocesses that run with the same OS privileges as the user who launched
Plexi. There is no process isolation, no syscall filtering, and no filesystem jail.

The security boundary works in two layers:

1. **Manifest declaration** — an app lists every capability it needs in
   `manifest.toml` under `[capabilities]`. The host refuses any protocol command
   that requires a capability the manifest did not declare.

2. **User consent** — on first use of a sensitive capability (or at install time
   for capabilities flagged `prompt: always`), the host shows a native permission
   dialog. The user's decision is recorded per workspace.

This is analogous to how browser extensions or macOS app entitlements work: the
manifest is a public contract, and the OS (here: the Plexi host) enforces that
contract at the API boundary. It is **not** analogous to a VM, container, or
WASM sandbox.

---

## What Capabilities Gate

Capabilities gate **protocol commands** — messages the app sends to the host over
the PGAP wire. The host validates the capability before processing the command.

| What is gated | Capability required |
|---|---|
| Reading files in the workspace | `fs.read` |
| Writing files in the workspace | `fs.write` |
| Outbound HTTP requests | `net.http` |
| Workspace secret access | `secrets.get` |
| Spawning typed pipes | `pipe.open` |
| Launching another app | `spawn.app` |
| Spawning panes | `panes.spawn` |
| Listing panes / reading pane state and content | `panes.read` |
| Focusing, closing, or sending input to panes | `panes.control` |
| Microphone capture | `audio.record` |
| Audio playback | `audio.playback` |
| Video decode | `video.playback` |
| LLM calls via Plexi AI broker | `ai.query` |
| MIDI input | `midi.in` |
| MIDI output | `midi.out` |
| Shell command execution via StreamProcess | `terminal.bindings` |
| Native file picker dialog | `fs.pick` |

Commands that require **no capability**: `ListAudioDevices`, `ListMidiDevices`,
`CopyToClipboard`, `MeasureText`, `StatusSummary`, `SaveAppState`, `Log`,
`ScheduleRender`, `SetMinSize`, `CloseSelf`, `PushNav`, `PopNav`,
`SetMouseTracking`.

### No ambient socket access

App subprocesses do **not** inherit `PLEXI_SOCKET`. Only terminal PTY panes get
it, because a human is typing there. An app that wants to see or drive other
panes must go through capability-gated PGAP requests (`panes.read` /
`panes.control`) — there is no ambient `plexi` CLI route from inside an app
process.

Be clear about what this is: it removes the **ambient grant**, it is not
isolation. Apps are still reviewed native processes, not sandboxed ones. A
malicious native process can find the socket path on disk and connect to it
directly. The socket file itself is the user's, with the user's permissions —
removing the env var stops well-behaved apps from quietly acquiring host
control, nothing more.

---

## What Is NOT Sandboxed

Because apps run as native Python subprocesses, a malicious or compromised app
that bypasses the protocol layer entirely retains the following access:

- **Full filesystem** — the app's Python process inherits the user's filesystem
  permissions. It can read and write any path the user can, regardless of declared
  capabilities.
- **Full network** — the app can open arbitrary sockets using standard Python
  libraries, regardless of whether `net.http` was declared.
- **Subprocess spawning** — the app can use `subprocess`, `os.system`, or
  `asyncio.create_subprocess_exec` directly, bypassing `terminal.bindings`.
- **Environment variables** — the app inherits the user's shell environment,
  including any secrets exposed via env vars.
- **Other processes** — the app can interact with other processes on the machine
  via signals, shared memory, or IPC.

The capability system prevents accidental misuse and provides a clear audit trail.
It does **not** prevent a deliberately hostile app from ignoring the protocol and
using Python's standard library directly.

---

## Shell Execution Audit

The full inventory of every shell execution path in the host (classified by trust
source) is maintained in [`docs/security/shell-execution-inventory.md`](security/shell-execution-inventory.md).

The key invariant: **no app-reachable `sh -c` path may exist without a capability
gate and a denial test.** The `terminal.bindings` capability is the only
app-facing path that executes user-visible shell commands, and it has both.

---

## Practical Guidance for Users

**Treat Plexi apps like shell scripts, not like browser tabs.**

- A browser tab is isolated: a malicious page cannot read your files.
- A Plexi app is not isolated: a malicious app can read your files, write to your
  disk, and make network requests — even without declaring capabilities — because
  it runs as a native subprocess.

Rules of thumb:

1. **Install only apps from sources you trust.** The manifest is a contract, not
   a constraint. A hostile author can write a manifest that declares no capabilities
   and still use the Python stdlib to do anything the user's account can do.

2. **Review the manifest before installing.** `manifest.toml` is the app's public
   capability declaration. An app that requests `terminal.bindings` or `net.http`
   can run shell commands or make outbound requests on your behalf.

3. **Workspace scoping is a soft boundary.** `fs.read` and `fs.write` are scoped
   to `workspace_root` in the protocol. A rogue app that bypasses the protocol
   has no such scope limit.

4. **Prefer open-source apps.** Reviewing the Python source is the most reliable
   way to know what an app does.

---

## Future: WASM Sandbox

The long-term security model is **WASM-based process isolation**. The design goals:

- Apps compile to WASM modules and run inside a WASI-compatible sandbox.
- Syscall filtering enforced at the WASM boundary — no direct OS access.
- Capabilities map to WASI capabilities: `fs.read` becomes a scoped WASI
  preopened directory, `net.http` becomes a WASI socket grant, etc.
- The host remains Rust; the app runtime switches from Python subprocesses to
  a WASM executor (e.g. Wasmtime).

This is a multi-quarter effort. v1 ships with the consent + audit model explicitly
because the alternative — shipping without documentation — implies safety that
does not exist.

When the WASM sandbox ships, the security model page will be updated. Until then,
the information on this page is current and complete.

---

## Summary Table

| Property | v1 (Python subprocess) | Future (WASM) |
|---|---|---|
| Process isolation | No | Yes |
| Filesystem jail | No | Yes (WASI preopens) |
| Network restriction | No | Yes (capability-granted) |
| Syscall filtering | No | Yes |
| Capability gating | Yes (protocol layer) | Yes (WASI layer) |
| Consent audit | Yes | Yes |
| Trust model | Consent + audit | Sandbox + consent |

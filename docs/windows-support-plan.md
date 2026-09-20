# Windows Support — Bringup Plan

Status: active
Stint: none — execution tracked on branch `feature/windows-v1-bringup` (PR #1604 replayed)

## Destination

`plexi.exe` builds, links, launches a real window, and serves its CLI on
Windows x86_64 from the same source tree that ships on macOS. macOS behavior is
unchanged: every Windows affordance is added behind `cfg(...)`, never by
deleting a macOS path.

Windows is a **third host platform**, alongside the Linux bringup
(`docs/linux-support-plan.md`, on `feature/linux-alpha-bringup`) — not a port. The three
scope models, the CLI surface and the pane model are platform-neutral already;
only the platform seam (IPC, secrets, menus, process control, shell probes)
is macOS-shaped and needs Windows arms.

Scope is the **v1 cut**: terminal host + scriptable CLI. Assistant, apps, SDK
and marketplace are explicitly out — they must compile, not work.

## Relationship to PR #1604

The original port (zachristmas/PLEXI, 46 commits) was written against alpha at
`92f752b3` (v0.0.514). Alpha has since moved ~1600 commits and restructured
nearly every file that port touched:

| Then | Now |
|---|---|
| `src/process_app/` | `src/app/` |
| `src/cli.rs` | `src/cli/` |
| `src/typed_pipes.rs` | `src/host/typed_pipes.rs` |
| `src/keys.rs`, `src/shell.rs` | `src/host/keys.rs`, `src/host/shell.rs` |
| `src/secrets.rs`, `src/workspace_secrets.rs` | `src/workspace/secrets/` |
| `src/overlays.rs`, `src/widgets.rs` | `src/overlays/`, `src/ui/` |

A rebase or merge would have produced modify/delete conflicts on nearly every
file and silently carried stale code forward, so the port was **replayed**
against the current tree instead. The original tip is preserved as the local
branch `windows-port-archive-v0` and the tag `windows-port-pre-replay`
(`393aeffa`), which is the reference for anything not yet replayed.

## Verification environment

Windows GUI behavior cannot be verified from the Linux development container.
What *is* verifiable here, and what proves it:

```bash
# Native MSVC target — the same toolchain windows-latest uses. `build` rather
# than `check` because linking a PE against the MSVC CRT is its own failure
# mode, and it is the half of the CI job a `check` does not cover.
cargo xwin build --bin plexi --target x86_64-pc-windows-msvc

# GNU target — a second opinion, faster to iterate against.
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo check --bin plexi --target x86_64-pc-windows-gnu
```

Success signal: `Finished`, with **zero errors and zero warnings**. A warning
here is not cosmetic — the dead-code warnings are how a subsystem silently
having no Windows arm shows up (see Phase 2).

Prerequisites, from a Debian container:

```bash
rustup target add x86_64-pc-windows-msvc x86_64-pc-windows-gnu
sudo apt-get install -y gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 clang llvm lld
cargo install cargo-xwin --locked
```

`cargo test --bin plexi` cannot run on a Windows target from here. It is run on
the Linux host instead, which shares the `not(target_os = "macos")` code paths
with Windows. Note that alpha does not currently compile for Linux either — see
"Known blockers" below.

## Phase 1 — Compile ✅

Clean on both Windows targets, and `x86_64-pc-windows-msvc` links a real
`plexi.exe` (~146 MB debug PE) — codegen and the link step, not just type
checking.

The platform seam:

- **`src/platform/ipc.rs`** — the host↔CLI transport. Unix keeps
  `UnixStream`/`UnixListener` as plain aliases; Windows gets Win32 named pipes
  at `\\.\pipe\plexi-<channel>`, mirroring the `~/.plexi-<channel>` profile
  split so each channel stays an isolated instance. `endpoint_in()` is the
  single derivation of "which address belongs to this channel", so the host's
  bind and the CLI's connect cannot drift.
- **Peer identity** — `GetNamedPipeClientProcessId`. This is the only
  trustworthy sender identity for `Notify`; a `None` fallback would have made
  every notification attribute to "outside pane".
- **Read timeouts** — named pipes have no `SO_RCVTIMEO`, so `set_read_timeout`
  is honoured by polling `PeekNamedPipe` and only issuing a `ReadFile` once
  bytes are known to be buffered. A CLI that cannot time out is a hung
  terminal.
- **Process detach** — `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, the
  creation-flag analogue of `setsid()`; there is no fork to hook.
- **Liveness** — `OpenProcess` + `GetExitCodeProcess`; no signal-0 equivalent.
- **`typed_pipes`** — overlapped named pipe. The overlap is load-bearing: it
  is what lets the drain thread poll for a client while still observing
  `shutdown`, so an app that never connects cannot wedge `close()` → `join()`.
- **Shell** — `$SHELL` → `pwsh` → `powershell` → `%ComSpec%`.
- **wgpu** — the base manifest entry names only `metal`; a
  `cfg(target_os = "windows")` table adds `dx12`, without which
  `create_surface` reports `NoAvailableAdapter` and no window ever opens.

## Phase 2 — Secrets ✅

Windows secrets live in the Credential Manager (`CredentialManager` in
`src/workspace/secrets/store.rs`), the Win32 analogue of `MacKeychain`.

Two honest differences from macOS, both documented at the impl:

- **No index sidecar.** `CredEnumerateW` takes a wildcard filter, so
  `list_with_prefix` and `scan_accounts` read the backend directly and cannot
  go stale against it.
- **`add_new` is read-then-write, not atomic.** `CredWriteW` has no create-only
  flag, so unlike `SecItemAdd` the duplicate check cannot be pushed into the
  backend. The residual race is real and is called out rather than papered
  over.

Without this the whole subsystem is dead on Windows — `plexi secret`,
`plexi ai onboard`, the OpenRouter lookup, terminal env injection and the
Secrets app all take the "no secret store on this platform" branch. That is
how ~35 dead-code warnings pointed at it.

## Phase 3 — CI ⏳

`.github/workflows/build-windows.yml` compiles `plexi.exe` on `windows-latest`
(`cargo check`, then `cargo build`). Nothing else in CI builds for Windows —
the release workflow is macOS-only — so a `cfg(unix)` assumption landing on
alpha is invisible until someone tries.

The file is currently staged at `.github/workflows-pending/build-windows.yml`
because the token that pushed this branch lacks GitHub's `workflow` OAuth
scope. See that directory's README for the two commands that activate it.

## Phase 4 — Keyboard ⛔ blocked on a product decision

**Host shortcuts currently shadow terminal control codes on Windows and
Linux, and every `exact: true` binding silently never fires.**

egui maps `Modifiers::COMMAND` to ⌘ on macOS but to **Ctrl** everywhere else.
Plexi's host shortcuts are all defined with `cmd()` (= `COMMAND`), so off
macOS every one of them becomes a bare `Ctrl+<key>` chord — exactly the
namespace a terminal uses for control codes (Ctrl+I = Tab, Ctrl+R =
reverse-search, Ctrl+W = delete-word, Ctrl+[ = ESC). `poll_actions` runs
before the terminal view reads input and consumes the event, so neovim and
readline never receive those keys.

Separately and verifiably, `modifiers_match_exact` in `src/host/keys.rs`
compares `actual.ctrl == pattern.ctrl`. Off macOS the runtime value for a Ctrl
press is `{ctrl: true, command: true}` while a `Modifiers::COMMAND` pattern is
`{command: true}`, so the comparison always fails. Reproduced with a throwaway
unit test on Linux:

```
exact match failed: actual=Modifiers { ctrl: true, command: true }
                    pattern=Modifiers { command: true }
```

All 14 `exact: true` bindings — pane navigation, pane swap, splits, rename,
close-context — are dead off macOS today.

**Why this is not fixed here.** The two are one coupled problem, not two bugs.
macOS has seven modifier tiers (`cmd`, `cmd_shift`, `cmd_ctrl`, `cmd_alt`,
`cmd_shift_alt`, `cmd_ctrl_alt`, `cmd_shift_ctrl`). Off macOS, with bare Ctrl
reserved for the terminal, only three chords are available: `Ctrl+Shift`,
`Ctrl+Alt`, `Ctrl+Shift+Alt`. Seven tiers do not fit in three, so some
shortcuts must share a chord or change key — and `cmd()` and `cmd_ctrl()`
already collapse onto the same chord, which is why `navigate_left` (Cmd+H) and
`swap_pane_left` (Cmd+Ctrl+H) become the same Windows chord.

Fixing `modifiers_match_exact` alone would make that collision *fire* rather
than stay dead, and the table's sort order would hand the win to
`swap_pane_left` over `navigate_left` — an accidental answer to a question
nobody has decided. PR #1604 solved this for four tiers (commit `126a4710`,
preserved on `windows-port-archive-v0`); the seven-tier version needs a
deliberate keymap, not an extrapolation.

**What the decision needs to cover:** which host shortcuts keep a chord on
Windows/Linux, which move key, and which are dropped to the command palette.
Once that exists, the implementation is small: `cfg`-split the `cmd*()` helpers
in `src/host/keys.rs`, fix `modifiers_match_exact` for non-macOS, and cover it
with `poll_actions` unit tests — which run headlessly on Linux and therefore
prove the Windows path.

## Phase 5 — Not yet replayed

Present on `windows-port-archive-v0`, not carried forward. None block the
compile gate; all are v1-relevant except where noted.

| Area | Archive commits | Note |
|---|---|---|
| ⌘/⇧ glyph swap in shortcut hints | `a19d44c2`, `55b7bbb8` | Blocked with Phase 4 — the hints must render whatever the keymap decides. |
| Inno Setup installer + PowerShell install/uninstall | `5e86c9c7`, `6318eb59`, `5cf07446`, `a03d9d66` | Needs a Windows machine to test; `installer/`, `scripts/*.ps1`. |
| App icon embedded in `plexi.exe` (`winresource`) | `ca0b84d7` | Cosmetic. |
| Bundled Python runtime | `97f14a2b` | Apps/SDK — **out of v1 scope**. |
| Per-pane cwd sidecar file | `818996ac` | Windows cannot read a shell's cwd from the OS; the prompt hook writes it to a file. Needed for context-root and pane-cwd features. |
| `.py` app launch via `python.exe`, console-window hiding | `b1608fa1`, `e0128bc6` | Apps — **out of v1 scope**. |

## Known blockers

- **Alpha does not compile for Linux.** `src/app/mod.rs` uses
  `UnixStream::peer_cred`, still unstable
  (`peer_credentials_unix_socket`). `feature/linux-alpha-bringup` fixes it with
  a direct `SO_PEERCRED` `getsockopt`. Until that lands, `cargo test --bin
  plexi` cannot run on a Linux box without locally applying that fix, which
  makes the shared non-macOS code paths harder to test than they should be.
- **No Windows GUI verification.** Everything past "it compiles and links" —
  window creation through dx12, ConPTY behavior in a real pane, Credential
  Manager round-trips, the installer — needs a human on a Windows machine.

## Merge order with the Linux bringup

The two branches touch the same platform seam and will conflict textually,
trivially, in a predictable place: gates widened to
`any(target_os = "macos", target_os = "linux")` on one branch and
`any(target_os = "macos", windows)` on the other resolve to the union of all
three. The `src/media/video.rs` dead-code fix is byte-identical on both
branches by design.

Landing Linux first is cheaper: it fixes the compile blocker above, so the
Windows branch can be tested rather than only checked.

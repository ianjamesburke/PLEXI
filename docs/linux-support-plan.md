# Linux Support — Bringup Plan

Status: active
Stint: none yet — execution tracked on branch `feature/linux-alpha-bringup`

## Destination

`plexi` builds, links, launches a real window, and serves its CLI on Linux
x86_64 (X11, Mesa/llvmpipe acceptable) from the same source tree that ships on
macOS. macOS behavior is unchanged: every Linux affordance is added behind
`cfg(target_os = ...)`, never by deleting a macOS path.

Linux is a **second host platform**, not a port. The three scope models, the
CLI surface, the pane model, and the app runtime are platform-neutral already;
only the platform seam (`src/platform/`, secrets, menus, video decode, shell
probes, window activation) is macOS-shaped and needs Linux arms.

## Why this document is a contract, not a checklist

Every phase below names the exact command an agent runs, the string that proves
it passed, and what the failure looks like. No phase is "done" because the code
looks right — it is done when its command emits its success signal on this
machine. Progress state lives on the branch and in `git log`, never here.

## Verification environment

The reference machine for this bringup:

- Debian 13 (trixie), x86_64, no GPU.
- X11 via `Xvfb` on a display exported in `DISPLAY`; viewable over VNC.
- Vulkan through Mesa **lavapipe** (`DRIVER_ID_MESA_LLVMPIPE`), with
  `VK_KHR_xlib_surface` present. This is what makes a headless VM able to run
  the real wgpu render path rather than only the headless renderer.
- Rust from rustup (`source ~/.cargo/env`), ≥ 1.91.

Confirm the environment before blaming the code:

```bash
echo "$DISPLAY"                 # must be non-empty
xdpyinfo | head -3              # must print "name of display"
vulkaninfo --summary | grep -i llvmpipe   # must print a deviceName
```

Failure shape: an empty `DISPLAY` or a `vulkaninfo` that lists zero devices
means Phase 3 will fail at surface creation with `NoAvailableAdapter`, and that
is an environment fault, not a code fault.

## Phase 0 — Host prerequisites

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config cmake \
  libssl-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libasound2-dev libfontconfig1-dev libfreetype6-dev libgtk-3-dev \
  libudev-dev libegl1-mesa-dev libgles2-mesa-dev \
  mesa-vulkan-drivers libgl1-mesa-dri vulkan-tools python3-dev
```

Why each group is load-bearing:

- **X11 + xkbcommon + wayland** — winit's Linux backends. Missing headers fail
  the *link*, not the compile, so they surface late and look like an unrelated
  error.
- **ALSA (`libasound2-dev`)** — `cpal` and `rodio` have no Linux backend
  without it; the build fails inside `alsa-sys`' build script.
- **GTK 3** — `rfd`'s Linux file dialog.
- **Mesa Vulkan + `vulkan-tools`** — the runtime adapter for Phase 3 and the
  `vulkaninfo` probe that distinguishes a code fault from a driver fault.

Success signal: `apt-get` exits 0 and `pkg-config --exists xkbcommon alsa gtk+-3.0`
exits 0. Failure: any `Package ... was not found` from a `*-sys` build script
during Phase 1 names the missing `-dev` package directly — install it and rerun.

## Phase 1 — Compile

Goal: `cargo check --bin plexi` is clean on Linux.

```bash
source ~/.cargo/env
cargo check --bin plexi --message-format short
```

Two classes of work land here.

**1a. Manifest.** `Cargo.toml` currently declares Apple-only choices in
target-neutral position. Each needs a Linux counterpart in a
`[target.'cfg(target_os = "linux")'.dependencies]` table or a widened feature
list:

- `wgpu` enables only the `metal` backend — Linux needs `vulkan` (and `gl` as
  a fallback), or `create_surface` finds no adapter at runtime even though the
  build is green.
- `eframe` runs `default-features = false`, which drops winit's `x11` /
  `wayland` features; they must be re-enabled for Linux.
- `notify` enables `macos_fsevent` with defaults off. Linux needs the inotify
  backend enabled explicitly for the hot-reload watcher.
- The `objc2*` / `security-framework` / `core-foundation` block is already
  correctly scoped under `cfg(target_os = "macos")` and must stay there.

**1b. Platform seam.** Give every macOS-only module a Linux arm rather than
removing it. The modules that are macOS-gated in `src/platform/mod.rs`
(`app_nap`, `finder_service`, `macos_menu`) plus the Keychain-backed secrets
store, the AVFoundation video decoder, the login-shell probe, and the window
activation-policy hook are the full seam. Grep for `target_os = "macos"` to
enumerate it; do not trust a frozen list.

Rules for the arms:

- Prefer an *honest* Linux implementation where one is cheap and obvious
  (`/proc` reads, XDG paths, inotify).
- Where it is not, add a **no-op or explicit-error stub that logs at `info`**,
  per the root instrumentation rule. A stub that silently succeeds is worse
  than one that returns `Err` — it makes a missing capability look present.
- Never `#[allow(dead_code)]` a macOS helper into compiling on Linux. If it has
  no Linux caller, gate the item itself.

Success signal: `cargo check --bin plexi` prints `Finished` with zero `error:`
lines. Failure: `error[E0433]`/`E0432` naming an `objc2`/`security_framework`
path means a macOS item escaped its gate; a `*-sys` build-script failure means
a Phase 0 package is missing.

## Phase 2 — Link and release build

`cargo check` does not link, and per the root testing contract neither
`cargo test` nor bare `cargo build` compiles with `-D warnings` or the
`cfg(not(test))` paths. Both of the following must pass:

```bash
cargo build --bin plexi                       # links the debug binary
just build                                    # release, -D warnings, the install path
```

Success signal: `target/release/plexi` exists and `target/release/plexi --version`
prints the version from `Cargo.toml`. Failure: an `undefined reference to`
during link is a missing Phase 0 system library; a `dead_code` denial under
`just build` is the classic gap where an item's last non-test caller was
macOS-gated away — gate the item, do not allow the lint.

## Phase 3 — Launch a window

```bash
DISPLAY=:<n> target/release/plexi host start
DISPLAY=:<n> target/release/plexi host status
```

Then prove the pixels exist through the sanctioned capture path — never an
OS-level screen grab:

```bash
plexi host screenshot --output /tmp/plexi-linux-boot.png
```

Success signal: `host status` reports a running host, `plexi.log` under
`~/.plexi-<channel>/` shows the wgpu adapter line naming `llvmpipe`, and the
PNG is a non-trivial image of the Plexi UI (read it; a uniform-black PNG is a
failure that a byte-size check would pass).

Failure shapes worth naming in advance:

- `NoAvailableAdapter` — Phase 1a's wgpu backend feature did not take effect.
- Process exits immediately with no window — `PLEXI_SOCKET` is set because the
  launching shell is itself inside a Plexi pane. `unset PLEXI_SOCKET` first.
- Window opens then hangs on first paint — llvmpipe is slow, not stuck; allow
  a generous first-frame budget before calling it a hang.

## Phase 4 — CLI smoke

The CLI is the platform-neutral surface, so it is the cheapest broad signal
that the Linux host is actually serving. Run the scripted check:

```bash
bash scripts/linux-smoke.sh
```

It drives, in order: `--version`, `doctor`, host start, host status, a pane
spawn, `pane state` on that pane, a screenshot, host shutdown, and a second
host quit from the window itself — asserting a success signal at each step and
exiting non-zero at the first failure. Each step prints `ok: <step>` on success
and `FAIL: <step>` with the captured command output on failure, so a red run
names its own phase.

Both quit steps assert the same two things, because quitting on Linux means
both: the process is gone, and `notify.sock` is unbound. The window quit is
driven with `xdotool` (skipped, not failed, where there is none) and needs no
window manager — the quit hotkey and the WM's close button reach eframe as the
same `CloseRequested`.

## Phase 5 — The standing gates

Linux must pass the same pre-push contract as macOS
(`src/testing/TESTING.md`). All three:

```bash
just build
cargo test --bin plexi
cargo clippy --bin plexi -- -D warnings
```

Tests that assert macOS-specific behavior (Keychain round-trips, AVFoundation
decode, menu construction) stay macOS-gated; a Linux stub gets its own test
asserting the *stub's* contract (returns the documented error, logs once), not
a skipped test. A test that is silently absent on Linux is invisible rot.

## Phase 6 — Install

`install.sh` and `scripts/install.sh` take a Linux branch rather than
refusing. Same script, same channel/suffix rules, same profile seeding; only
the artifact and its placement differ, because Linux has no `.app` bundle.

| | macOS | Linux |
|---|---|---|
| Build | `cargo bundle --release` | `cargo build --release` |
| Installed binary | `/Applications/Plexi<Cap>.app/Contents/MacOS/plexi<suffix>` | `$XDG_DATA_HOME/plexi<suffix>/bin/plexi<suffix>` |
| PATH entry | `/usr/local/bin/plexi<suffix>` (sudo) | `$HOME/.local/bin/plexi<suffix>` (no sudo) |
| Launcher | `Info.plist` + `lsregister` | `.desktop` entry + hicolor icon |
| Signing | `codesign` with "Plexi Dev" | n/a |

Both platforms resolve every later step — the channel-routing shim, the
non-main symlink, completions, core-pack seeding — through a single
`$stable_bin` variable, so the two branches cannot drift apart on where the
binary actually is.

Two Linux details that are easy to get wrong:

- **The binary keeps its channel suffix on disk.** `config_dir_name()` derives
  the profile from `current_exe()`'s basename, so a binary installed as plain
  `plexi` reads `~/.plexi/` no matter which channel built it.
- **`rsync` is not guaranteed.** It ships with macOS but not with a minimal
  Debian or Fedora install, and every use in the installer is "replace this
  directory with that one". `sync_tree` falls back to `cp -R`, so the
  installer does not hard-require a package the user may not have.

`PLEXI_BIN_DIR` overrides the PATH entry's directory. The installer warns when
that directory is not on `$PATH` instead of silently installing a command the
shell cannot find. `scripts/uninstall.sh` reverses all of it, including the
`.desktop` entry and the icon.

## Non-goals for v0

Explicitly out of scope. Each is a stub or an unsupported path, and the binary
must say so rather than pretend:

- **AVFoundation parity.** No Linux hardware video decode. The video pane
  reports "unsupported on this platform"; it does not fall back to a silent
  black frame.
- **Distro packaging.** No `.deb`, `.rpm`, AUR recipe, Flatpak or AppImage,
  and no `cargo-bundle` equivalent. `install.sh` builds from source and
  installs into the user's home (Phase 6); there is nothing to publish to a
  package repository.
- **App Nap.** A macOS-only energy concept with no Linux analogue — the Linux
  arm is a documented no-op, not an emulation.
- **Keyring-backed secrets.** No libsecret / gnome-keyring integration in v0.
  Linux resolves `system_store()` to a file-backed `SecretStore` (`FileStore`,
  in the secrets store module) writing a `0600` JSON file in the channel
  profile dir. That is strictly weaker than the macOS Keychain — same-user
  readable, not encrypted at rest — and the store says so in the log the first
  time it is touched. It exists because the alternative is no store at all,
  which makes `plexi ai onboard`, the OpenRouter key lookup, terminal env
  injection and the Secrets app dead code on Linux.
- **Finder service / macOS menu bar / dock integration.**
- **Wayland.** X11 only for v0. Wayland may work incidentally; it is not
  verified and not claimed.
- **Channel promotion on Linux.** `scripts/install.sh <channel>` installs any
  channel (`plexi-alpha`, `plexi-beta`, `plexi-pr-<N>`) with the same suffix
  rules as macOS, but the promotion and release-tagging scripts around it are
  untested off macOS.
- **CI.** No Linux job added to the pipeline in v0.

## Definition of done — "alpha runs on this Linux VM"

All of the following are simultaneously true on the reference machine:

1. `just build` is green with `-D warnings`.
2. `cargo test --bin plexi` is green.
3. `cargo clippy --bin plexi -- -D warnings` is green.
4. `plexi host start` opens a real window on the X display and the host stays
   up; `plexi host status` agrees.
5. `plexi host screenshot` produces a PNG that, when read, shows the Plexi UI —
   chrome and at least one pane — not a blank surface.
6. `bash scripts/linux-smoke.sh` exits 0 end to end — against the *installed*
   binary, not just the one in `target/release/`.
7. Every macOS capability with no Linux implementation is reachable only
   through a stub that logs at `info` and, where it must fail, returns an
   error naming the platform — verified by grepping the log after a smoke run.

8. `bash scripts/install.sh <channel>` completes on a machine with no
   `rsync` and no `sudo`, and the resulting `plexi` on `$PATH` starts a host.

Anything short of all eight is "Linux compiles", which is a different and much
weaker claim.

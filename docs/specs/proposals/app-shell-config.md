# Plexi App Spec: `shell-config`

**Status:** v1 draft
**Date:** 2026-04-14
**Type:** External Python app (draw protocol)
**Install path:** `~/.plexi-alpha/apps/shell-config/`
**Priority:** P3 (idea-tier; not a shipping blocker)

---

## Motivation

Users accumulate small shell tweaks over time — aliases, PATH tweaks, one-off env vars, sourced helper scripts. Managing these by hand means editing `~/.zshrc` (or a dotfile it sources), which is risky: one bad line breaks every new shell, and there's no obvious "off switch" when something misbehaves.

Plexi can do better by owning a **sibling** shell config that layers on top of the user's real rc, never inside it. The host already uses this pattern: `src/shell.rs` hijacks `ZDOTDIR` for Plexi-launched shells and sources the user's original `~/.zshrc` from its own synthetic `.zshrc` (see `src/shell.rs:176–189`). This app exposes that layering to users as a managed surface:

- **Add** an alias / env / PATH / source entry from inside Plexi.
- **Enable / disable** individual entries without editing any file by hand.
- **Nuke** the entire addon layer with `rm -rf ~/.plexi-shell` and end up in exactly the state the user was in before installing the app.

### The hard invariant

**The shell-config app MUST NEVER write to any user-owned dotfile.** That includes but is not limited to:

- `~/.zshrc`, `~/.zshenv`, `~/.zprofile`, `~/.zlogin`, `~/.zlogout`
- `~/.bashrc`, `~/.bash_profile`, `~/.profile`
- `~/.config/fish/config.fish`
- Anything under `~/dotfiles/` (users who keep their real config in a dotfiles repo that `~/.zshrc` sources — that repo is sacred).

All app state lives under `~/.plexi-shell/`. This invariant is what makes the app safe to try and safe to throw away.

### Why not just "edit ~/.zshrc for me"?

- It's an irreversible mutation of a file the user owns and may have under version control.
- Many users have a thin loader `~/.zshrc` that sources a shared `~/dotfiles/zshrc` across machines. Editing that loader is fine locally but silently non-portable; editing the dotfiles repo is out of scope for a per-machine app.
- "Append a line and pray" is how shell config rot happens. A managed addon directory that we own end-to-end is both safer and simpler to reason about.

---

## Non-goals

v1 is deliberately narrow. The following are **explicitly out of scope**:

- **Not a dotfile sync tool.** No cloud, no git, no "push to machine B." (Deferred to v2+.)
- **Not a plugin manager.** Not trying to replace oh-my-zsh, zinit, antidote, prezto, zgenom, fig, etc. It coexists with them.
- **Not a prompt theming tool.** Don't touch `PS1`, starship, powerlevel10k, pure, spaceship, or any prompt framework. If the user has one of these, the app leaves it entirely alone.
- **Not a bash / fish config manager.** v1 is zsh-only. Other shells are detected and the app refuses to activate rather than misbehaving.
- **Does not implement `chpwd` hooks.** Directory-scoped env injection (the direnv pattern) is a separate P3 idea tracked elsewhere.
- **Does not modify the user's login shell.** `chsh` is never called.
- **Not a secrets manager.** Secrets go through Plexi's existing `SecretGet` / `SecretStore` API. This app manages shell init text, not secret values.
- **Does not edit `/etc/zshenv`, `/etc/zshrc`, or any system file.**

---

## Architecture

### ZDOTDIR, in one paragraph

zsh consults the environment variable `ZDOTDIR` at startup. If it's set, zsh reads its dotfiles (`.zshenv`, `.zprofile`, `.zshrc`, `.zlogin`, `.zlogout`) from that directory instead of `$HOME`. This means we can hand zsh a directory full of files we control, have our `.zshrc` first `source` the user's real `~/.zshrc`, then append whatever the app's config says, and the user's home dotfiles remain pristine.

### Layout

```
~/.plexi-shell/
├── zdotdir/                   # what we hand zsh via ZDOTDIR
│   ├── .zshenv                # sources ~/.zshenv if it exists
│   ├── .zprofile              # sources ~/.zprofile if it exists
│   ├── .zshrc                 # (1) sources ~/.zshrc, then (2) sources addons.zsh
│   ├── .zlogin                # sources ~/.zlogin if it exists
│   ├── .zlogout               # sources ~/.zlogout if it exists
│   └── addons.zsh             # generated from config.toml — the only file the app re-writes on every sync
├── scripts/                   # user-provided sourced scripts land here (copied on add)
│   └── *.sh
├── config.toml                # source of truth for what's enabled
└── state.json                 # runtime state (last sync time, last error, schema version)
```

**Invariants:**

1. Files under `zdotdir/` other than `addons.zsh` are written **once, at install**, and never touched again (they're tiny shims).
2. `addons.zsh` is **regenerated from scratch** on every sync. No merge logic, no "preserve manual edits." If you hand-edit it, your edit is lost on the next sync. This is a feature — it makes the config.toml the single source of truth.
3. The app writes nowhere outside `~/.plexi-shell/`. Period.

### Shim file contents (stable, written once)

`~/.plexi-shell/zdotdir/.zshrc`:

```sh
# Managed by Plexi shell-config. Do not edit by hand.
# The source of truth is ~/.plexi-shell/config.toml.

# 1. Load the user's real zsh config from $HOME, untouched.
if [ -f "$HOME/.zshrc" ]; then
  source "$HOME/.zshrc"
fi

# 2. Apply Plexi-managed addons on top.
if [ -f "${ZDOTDIR:-$HOME}/addons.zsh" ]; then
  source "${ZDOTDIR:-$HOME}/addons.zsh"
fi
```

The other shims (`.zshenv`, `.zprofile`, `.zlogin`, `.zlogout`) are one-liner `source $HOME/<same-file> 2>/dev/null || true` stubs. They exist so zsh still loads the user's real hooks when `ZDOTDIR` is redirected.

---

## Install / activation model

This is the biggest design question. Three options were considered:

1. **Global** — Plexi itself spawns all shells with `ZDOTDIR=~/.plexi-shell/zdotdir`. Applies to every pane, every session, always.
2. **Per-pane toggle** — Command palette: "Enable Plexi shell-config in this pane." Next shell spawned in that pane gets the addon; existing shells are unchanged.
3. **Opt-in export in the user's rc** — User adds `export ZDOTDIR=~/.plexi-shell/zdotdir` to their own `~/.zshenv`.

**Option 3 is rejected outright** — it requires editing a user-owned dotfile, which violates the hard invariant.

**v1 recommendation: Option 1 (global, opt-in with clear off-switch).**

Plexi already owns shell spawning (`src/shell.rs:build_env`). When the shell-config app has been enabled once (a flag in Plexi's own config, not the user's rc), Plexi sets `ZDOTDIR=~/.plexi-shell/zdotdir` on every shell it spawns. Existing shells stay unchanged — only new ones pick up the addon.

Rationale:

- Per-pane (Option 2) sounds nice but adds state-tracking complexity without a real use case for v1. "Which panes have addons active" is a weird mental model; it's easier to just say "Plexi shells either have the addon layer or they don't."
- Global scope only touches **Plexi-spawned shells** — a plain Terminal.app or iTerm2 window outside Plexi still uses the user's real `~`. This is an important property; the app is scoped to Plexi, not to the machine.
- The user can turn it off with a single command (`shell-config:disable`) and new shells revert instantly.

Enabling the app writes exactly one line to `~/.plexi-alpha/config.toml` (Plexi's own config — not the user's dotfiles):

```toml
[shell_config]
enabled = true
```

Plexi's `build_env()` reads this flag; when true, it sets `ZDOTDIR` on spawned shells. When false, it doesn't. That's the entire activation mechanism.

**If the user already has `ZDOTDIR` set** in their environment when Plexi starts, the app refuses to enable and shows a clear error explaining why. See Edge Cases.

---

## Config format

TOML, for consistency with `manifest.toml`, `permissions.toml`, and the rest of Plexi. Single file at `~/.plexi-shell/config.toml`.

```toml
# ~/.plexi-shell/config.toml
# Source of truth for Plexi-managed shell addons.
# Regenerates ~/.plexi-shell/zdotdir/addons.zsh on every sync.

schema_version = 1

[[aliases]]
name = "ll"
command = "ls -lAh"
enabled = true

[[aliases]]
name = "gco"
command = "git checkout"
enabled = true

[[env]]
name = "EDITOR"
value = "nvim"
enabled = true

[[env]]
name = "PAGER"
value = "less -R"
enabled = true

[[path]]
# prepended to PATH, in order of appearance
dir = "$HOME/.local/bin"
enabled = true

[[path]]
dir = "$HOME/bin"
enabled = false

[[source]]
# must be a script already living under ~/.plexi-shell/scripts/
path = "work-helpers.sh"
enabled = true
```

**Four entry types, no more, for v1:** `aliases`, `env`, `path`, `source`. Anything else the user wants goes in a `source` script they author themselves and drop into `~/.plexi-shell/scripts/` (the app's `add-script` command copies a file into that directory).

**Why TOML and not JSON:** the user reads and edits this file. Comments matter. Plexi standard.

**Why "enabled" flags and not "just delete the entry":** the point of the app is toggling things on and off to debug "which line broke my shell," without losing the line. Keep it, flip the flag, re-sync.

### Generated `addons.zsh`

`sync` reads `config.toml` and writes `~/.plexi-shell/zdotdir/addons.zsh` from a template:

```sh
# GENERATED FILE. Do not edit by hand.
# Regenerated from ~/.plexi-shell/config.toml on every `shell-config:sync`.

# --- env ---
export EDITOR="nvim"
export PAGER="less -R"

# --- path ---
export PATH="$HOME/.local/bin:$PATH"

# --- aliases ---
alias ll='ls -lAh'
alias gco='git checkout'

# --- sources ---
if [ -f "$HOME/.plexi-shell/scripts/work-helpers.sh" ]; then
  source "$HOME/.plexi-shell/scripts/work-helpers.sh"
fi
```

Disabled entries are omitted entirely (not commented out). Write is atomic: write to `addons.zsh.tmp`, `fsync`, rename over `addons.zsh`.

---

## UI

The app uses the draw protocol. Draw commands used: `rect`, `text`, `frame_done`.

Three screens, navigated with Tab / number keys:

### 1. Status screen (default)

- Top bar: `shell-config` title, enabled/disabled indicator, last-sync timestamp, last-sync status.
- Detected shell (e.g. `zsh 5.9`). If non-zsh, shown in red with "zsh required" message.
- Current `ZDOTDIR` value in the user's login env (if any) — shown red if set to something the app doesn't own.
- Count of enabled entries by type: `4 aliases / 2 env / 1 path / 1 source`.
- Keyboard: `e` toggle enable/disable, `s` sync now, `1/2/3` jump to other screens.

### 2. Entries screen

- Four horizontally-scrollable columns, one per entry type: Aliases | Env | Path | Source.
- Each row is `[x] name = value` (checkbox for enabled state).
- Keys: arrows to move, space to toggle, `d` delete, `a` add (opens modal), `s` save & sync.
- Add-alias modal: two `text` prompts (name, command). Returns to list on submit.
- Changes are staged in memory until save, so accidental toggles don't touch disk.

### 3. Raw screen

- Shows the current rendered `addons.zsh` (what new shells will actually see).
- Read-only preview. Scrollable. No edit affordance — all edits go through screen 2.

### Not included in v1 UI

No search, no fuzzy filter, no multi-select, no undo stack, no import from existing `~/.zshrc`. Those are the obvious v2 improvements once the core layer is trusted.

---

## Commands

Exposed through Plexi's command palette as `Command` events. All are responded to with a draw refresh and a `log` command describing the result.

| Command | Args | Behavior |
|---|---|---|
| `shell-config:enable` | — | Writes `shell_config.enabled = true` to Plexi's config, creates `~/.plexi-shell/` if missing, regenerates `addons.zsh`. Notifies the user that only newly-spawned shells pick up the change. |
| `shell-config:disable` | — | Flips the flag to false. `~/.plexi-shell/` is left in place. New shells spawn without `ZDOTDIR`. |
| `shell-config:sync` | — | Regenerates `addons.zsh` from `config.toml`. Safe to run repeatedly. |
| `shell-config:add-alias` | `name`, `command` | Appends an entry to `[[aliases]]`, syncs. |
| `shell-config:add-env` | `name`, `value` | Appends an entry to `[[env]]`, syncs. |
| `shell-config:add-path` | `dir` | Appends an entry to `[[path]]`, syncs. |
| `shell-config:add-source` | `path` (file on disk) | Copies file into `~/.plexi-shell/scripts/`, appends a `[[source]]` entry, syncs. |
| `shell-config:toggle` | `type`, `name` | Flips the `enabled` flag on a single entry, syncs. |
| `shell-config:remove` | `type`, `name` | Deletes an entry from config.toml, syncs. |
| `shell-config:status` | — | Returns a summary (used by the status screen and for scripting). |
| `shell-config:uninstall` | — | Confirms, sets enabled=false, deletes `~/.plexi-shell/` recursively. |

All commands are idempotent. `enable` on an already-enabled config is a no-op + info log.

---

## Capability declarations (`manifest.toml`)

```toml
[app]
id = "shell-config"
name = "Shell Config"
entry = "shell_config.py"
version = "0.1.0"
description = "Manage zsh addons (aliases, env, PATH, sourced scripts) without ever touching your ~/.zshrc."

[app.capabilities]
filesystem = "read_write"  # needs to read/write ~/.plexi-shell/**
terminal_write = false     # does not inject text into shell panes
file_types = []
network = false
```

Capability notes:

- The app's filesystem reach is advisory in v1 (see `docs/specs/subsystems/app-infrastructure.md` Capability system). The app itself must enforce the `~/.plexi-shell/` boundary in code and refuse to resolve any path outside its root. This is belt-and-suspenders: the invariant is safety-critical.
- No `network`. Ever. An app that manages shell init with network access is a supply-chain footgun.
- No `terminal_write`. The app does not paste into running shells; its effect is on future shells only.

---

## Uninstall story

```sh
rm -rf ~/.plexi-shell
```

Plus flipping `shell_config.enabled = false` in `~/.plexi-alpha/config.toml` (the app's `uninstall` command does both). After uninstall, the machine state is:

- `~/.plexi-shell/` — gone.
- User dotfiles (`~/.zshrc`, `~/.zshenv`, `~/dotfiles/*`, etc.) — **byte-for-byte identical** to before the app was ever installed. We never touched them.
- Plexi's own config — has a single `[shell_config]` section with `enabled = false`. A totally clean uninstall also removes that section; the command does this.
- New shells spawned by Plexi — unset `ZDOTDIR`, read `~/.zshrc` directly, behave exactly as they did before.

If any of these properties fail, it's a bug.

---

## Edge cases and failure modes

| Case | Behavior |
|---|---|
| User's login shell is not zsh (bash, fish, nu, etc.) | `enable` refuses, shows a clear error: "shell-config v1 supports zsh only. Detected: <shell>." Non-fatal. App still runs; just won't activate. |
| `$ZDOTDIR` is already set in the user's login env to something that isn't `~/.plexi-shell/zdotdir` | `enable` refuses. Error explains that the user has a custom zsh setup the app won't clobber, and points at the spec's "nested ZDOTDIR" section. v1 does not try to chain. |
| User already has `oh-my-zsh` / `prezto` / `zinit` / `starship` / `powerlevel10k` / `pure` / etc. | Works fine. These are configured inside `~/.zshrc` (or files it sources), which our shim sources first, before `addons.zsh`. The user's framework still "wins" for anything the app doesn't touch. The app never sets `PS1`, never re-`autoload compinit`, never reorders plugins. |
| User has a thin `~/.zshrc` that sources `~/dotfiles/zshrc` | Works fine. The app sources `~/.zshrc`, which sources `~/dotfiles/zshrc`, which does its thing. The app never looks at or writes to `~/dotfiles/`. |
| User edits `addons.zsh` by hand | Their edits are clobbered on the next `sync`. The file has a "GENERATED FILE" header warning them. This is the intended design. |
| `addons.zsh` contains a syntax error (because the user put a bad value in config.toml) | Every new shell prints an error at startup but continues — since the shim `source`s our addon after the user's real config, a broken addon does not break the shell entirely. Disable the offending entry or `disable` the whole app. |
| Plexi is upgraded and `shell_config.enabled=true` is preserved | New Plexi reads the same flag and spawns shells with `ZDOTDIR` as before. The app detects a schema version mismatch in `config.toml` on first launch and offers to migrate. v1 supports only `schema_version = 1`; migration logic is a stub. |
| User deletes `~/.plexi-shell/zdotdir/addons.zsh` manually | Next `sync` regenerates it. No harm. |
| User deletes `~/.plexi-shell/` entirely while enabled | Plexi still spawns shells with `ZDOTDIR=~/.plexi-shell/zdotdir`, which doesn't exist — zsh falls back to `$HOME` on its own. Shells still work. The app's status screen shows "not installed" and offers `enable` again. |
| `config.toml` is corrupt TOML | Load fails, app renders an error screen with the parse error, refuses to sync until the user fixes it or runs `shell-config:reset` (which backs up the corrupt file as `config.toml.broken-<timestamp>` and writes a fresh empty one). |
| Non-macOS / Linux | zsh on Linux works the same. The app is shell-specific, not OS-specific. v1 is tested on macOS but has no mac-only calls. |

---

## Security considerations

Shell init code runs in every new shell. A bad entry here has a large blast radius. The app must behave like it knows that:

1. **Own nothing outside `~/.plexi-shell/`.** Enforced by the app's own path resolver; every file operation passes through a helper that refuses any absolute path outside the root.
2. **Regenerate, never merge.** `addons.zsh` is rewritten from `config.toml` each sync, so the on-disk state can't drift from the user-visible state.
3. **No network capability.** No pulling snippets from a URL, no "install this preset from GitHub." If v2 adds sharing, it's explicit and routed through Plexi's capability gates with user confirmation per snippet.
4. **No shell evaluation of user-provided strings at add-time.** The app writes values verbatim into `addons.zsh` (quoted), but does not `eval` them itself. Any error shows up at next shell startup, scoped to a single shell session, not inside the app process.
5. **Alias / env names are validated** against `^[A-Za-z_][A-Za-z0-9_]*$` before being written. Garbage names would produce a broken `addons.zsh` and break every new shell in Plexi.
6. **Paths added to `$PATH` are not validated to exist.** That's a user choice (they may add dirs that only exist on some hosts). But dangerous entries like `.` or an empty string are refused.
7. **Never affects shells Plexi didn't spawn.** Terminal.app, iTerm2, tmux-outside-Plexi, SSH sessions — all untouched. The app's scope is strictly Plexi-spawned shells.
8. **Enable is loud, disable is quiet.** `enable` prints a banner in the app's status line and emits a warn-level Plexi log entry. Disabling or uninstalling is silent beyond a single info log. The user should always know when shell init is being modified.

---

## v1 MVP checklist

All testable. Shippable in a few days.

- [ ] `manifest.toml` declares `filesystem = "read_write"`, no network, no terminal_write.
- [ ] On first launch with no `~/.plexi-shell/`, the status screen shows "not installed" and `shell-config:enable` bootstraps the directory with all shims.
- [ ] `shell-config:enable` flips `shell_config.enabled = true` in Plexi's own config.toml; a freshly spawned pane inside Plexi has `ZDOTDIR=~/.plexi-shell/zdotdir` in its env.
- [ ] A freshly spawned pane inside Plexi still sees everything from the user's original `~/.zshrc` (aliases, functions, prompt, plugins).
- [ ] Adding an alias via `shell-config:add-alias` results in that alias being usable in the next Plexi-spawned shell, and not in any shell spawned before the add.
- [ ] Toggling an entry off and re-syncing makes it disappear from the next new shell.
- [ ] Running `shell-config:disable` and opening a new shell: `ZDOTDIR` is unset and the addons are not loaded.
- [ ] `shell-config:uninstall` deletes `~/.plexi-shell/` and sets enabled=false; running `diff` on `~/.zshrc` before install and after uninstall shows no changes.
- [ ] Refuses to enable if `$SHELL` is not zsh; refuses to enable if a user `$ZDOTDIR` is already set to a path other than ours.
- [ ] A syntactically-broken `addons.zsh` (induced via a bad `add-env`) does not prevent new shells from starting — the user's real `~/.zshrc` still runs first.

---

## Deferred to v2+

- **Sync / share.** Git-backed `~/.plexi-shell/config.toml`, team presets, import from a URL. Needs network capability and a trust model.
- **Bash and fish support.** Separate code paths, separate shims, separate test surface.
- **`chpwd` / direnv-style directory-scoped env injection.** Tracked as its own P3 idea; interacts with the existing `resolve_secret` directory walk.
- **Import existing snippets from `~/.zshrc`.** Read-only parse to pull out alias/export lines and offer to adopt them. Hard to do safely; easy to do badly.
- **Per-pane enable** (Option 2 from the activation section).
- **Preset library** — curated bundles of aliases/env for common workflows (git, rust, python).
- **Nested ZDOTDIR chaining** for users who already have a custom `ZDOTDIR`.
- **GUI multi-select / undo / diff-viewer before sync.**
- **Prompt theme integration** (starship / p10k config exposure). Possibly never — tread carefully.

---

## Implementation notes

- **Language:** Python 3 (matches the existing example app ecosystem). First line of every `.py` file: `from __future__ import annotations` per the project CLAUDE.md.
- **Entry point:** `shell_config.py`. Uses the bundled `plexi_sdk.py` like other examples.
- **Files in `examples/shell-config/`:**
  ```
  examples/shell-config/
  ├── manifest.toml
  ├── shell_config.py          # main app loop (init / render / key / command handlers)
  ├── config_store.py          # load/save config.toml, validate entries
  ├── addons_writer.py         # regenerates addons.zsh atomically
  ├── shell_detect.py          # detects $SHELL, zsh version, existing ZDOTDIR
  ├── plexi_sdk.py             # copied from the shared SDK
  └── tests/
      ├── test_addons_writer.py
      ├── test_config_store.py
      └── test_shell_detect.py
  ```
- **Install target:** `~/.plexi-alpha/apps/shell-config/` (follows the usual alpha-build convention).
- **Host-side change required:** `src/shell.rs::build_env()` reads `shell_config.enabled` from Plexi's config and sets `ZDOTDIR=~/.plexi-shell/zdotdir` when true and the directory exists. This is a ~10-line change gated behind a feature flag; it is the **only** change in the Plexi host for v1. If that change lands separately, the app can still be installed and iterated against; it just won't activate until the host side is wired.
- **Logging:** use `ctx.info` / `emit.info` for every state-changing command. Errors from the config parser go through `ctx.error` so they land in `~/.plexi-alpha/plexi.log` tagged `app::shell-config`.
- **Testing:** unit tests cover `addons_writer` (regeneration is deterministic given config.toml) and `config_store` (round-trip through TOML preserves everything). Manual E2E test per the MVP checklist is the acceptance bar for v1.

---

## GitHub issue draft

Ready to file with `gh issue create`:

```
Title: Shell config manager app (safe ZDOTDIR-addon pattern)
Labels: idea, P3
```

> Priority: P3 — idea-tier, not an MVP blocker. Filed so the safe-pattern research doesn't rot.

### Summary

A Plexi app that manages zsh addons (aliases, env vars, PATH additions, sourced scripts) without ever touching the user's own dotfiles. The app owns a sibling directory at `~/.plexi-shell/` containing a managed `ZDOTDIR` that first sources the user's real `~/.zshrc`, then layers a generated `addons.zsh` on top. Plexi spawns shells with `ZDOTDIR` pointing at this sibling dir when the app is enabled; otherwise shells behave exactly as they always have.

### Why

Users accumulate shell tweaks and have no safe way to toggle them. Editing `~/.zshrc` is destructive, non-reversible, and risky for users whose real config lives in a dotfiles repo. The ZDOTDIR-addon pattern is the known-safe approach — Plexi itself already uses it in `src/shell.rs` to layer its own `.zshrc` over the user's. This app surfaces that pattern as a user feature. **Hard invariant: the app must never write to `~/.zshrc`, `~/.zshenv`, `~/dotfiles/`, or any user-owned file. All state lives under `~/.plexi-shell/`, and uninstall is `rm -rf` of that directory.**

### Acceptance criteria

- [ ] `shell-config:enable` creates `~/.plexi-shell/zdotdir/` with a `.zshrc` that sources `~/.zshrc` then `addons.zsh`, and flips a flag in Plexi's own config so new Plexi-spawned shells use `ZDOTDIR=~/.plexi-shell/zdotdir`.
- [ ] A newly-spawned Plexi shell sees everything from the user's real `~/.zshrc` unchanged (plugins, prompt, aliases, functions).
- [ ] `shell-config:add-alias`, `add-env`, `add-path`, `add-source` append entries to `~/.plexi-shell/config.toml` and regenerate `addons.zsh` atomically.
- [ ] Each entry has an `enabled` flag that can be toggled from the app UI without deleting the entry; disabled entries do not appear in the generated `addons.zsh`.
- [ ] `shell-config:disable` reverts: new shells are spawned with no `ZDOTDIR` and read `~/.zshrc` directly.
- [ ] `shell-config:uninstall` deletes `~/.plexi-shell/` entirely; `diff` of `~/.zshrc` before install and after uninstall shows zero changes.
- [ ] App refuses to enable on non-zsh shells or when the user already has a `$ZDOTDIR` set to a path the app doesn't own.
- [ ] `manifest.toml` declares `filesystem = "read_write"`, no `network`, no `terminal_write`.

Full v1 spec in `docs/specs/proposals/app-shell-config.md`. Implement as an external Python app in `examples/shell-config/` following the app-infrastructure protocol in `docs/specs/subsystems/app-infrastructure.md`.

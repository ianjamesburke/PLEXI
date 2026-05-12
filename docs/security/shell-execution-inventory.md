# Shell Execution Inventory

Every `sh -c`, `Command::new`, and shell-string execution path in Plexi, classified
by trust source. Maintained as an authoritative audit document (issue #1177).

## Classification legend

| Class | Meaning |
|---|---|
| **internal-constant** | Hardcoded binary + args; no user or app input reaches the exec path |
| **user-authored-config** | String originates from user-owned config (`config.toml`, `.plexi/commands.toml`); the user is also the operator, so trust is equivalent to a shell alias |
| **app-requested** | String supplied by an app subprocess; requires a declared capability gate before execution |

---

## App-reachable paths

These paths are reachable by installed Plexi apps via the draw-command protocol.
Every entry **must** have a capability gate. The gate is the only trust boundary
between an app's runtime and the host OS.

### `DrawCommand::StreamProcess` → `sh -c`

| Field | Value |
|---|---|
| File | `src/process_app/routing.rs:553` |
| Class | **app-requested** |
| Capability gate | `Capability::TerminalBindings` (`terminal.bindings`) |
| Denial path | Returns `PlexiEvent::StreamEnd { exit_code: 1 }` immediately; subprocess never spawned |
| Logging | `log::warn!` on denial; `log::info!` with app id + command + channel on allow |
| Test | `stream_process_denied_without_terminal_bindings` in `src/testing.rs` |

An app that declares `terminal.bindings` in its manifest is explicitly trusted
to execute shell commands on the user's behalf. The capability prompt is shown
at first use and remembered per workspace. An undeclared app hits the denial
path — no subprocess is ever spawned.

---

## User-authored paths

These paths run commands that the user wrote themselves. No additional capability
gate is warranted — trust is identical to a shell alias or cron job.

### `plexi run` → `sh -c`

| Field | Value |
|---|---|
| File | `src/cli.rs:109` |
| Class | **user-authored-config** |
| Input source | `.plexi/commands.toml` `run` field — written by the workspace owner |
| Secret injection | Secrets are resolved by name from the workspace secrets store and injected as env vars. Secret values never appear in the command string itself. |
| App-reachable | No — this is a CLI subcommand invoked by the user directly, not by app protocol |
| Test | None required (user-authored); see secret resolution tests in `src/workspace_secrets.rs` |

`.plexi/commands.toml` is executable code. Any secret referenced must exist in
the workspace secrets store; missing secrets abort with a clear error rather than
substituting empty strings.

### Quick-note destination command templates → `zsh -c` / `sh -c`

| Field | Value |
|---|---|
| File | `src/pane_ops/create.rs:791` (`substitute_note_tokens_static`) |
| Class | **user-authored-config** |
| Input source | `[[quick_note.destinations]]` in `config.toml` — written by the user |
| User-supplied tokens | `{note}` and `{cwd}` — both routed through `shell_quote` before substitution |
| Shell-quote impl | POSIX single-quote wrapping (`'...'`) with `'` escaped as `'\''`; blocks `$(...)`, backticks, `\`, newlines, semicolons |
| App-reachable | No — apps cannot submit notes or select destinations |
| Tests | `src/pane_ops/create.rs:1274–1342` — 8 injection / escaping tests added by issue #1113 |

The command _template_ is trusted (user-authored config). Only the substituted
values are escaped. Do not add new substitution tokens that expand
arbitrary strings without routing them through `shell_quote`.

---

## Internal paths

These paths execute hardcoded binaries with caller-controlled (not user/app-controlled)
arguments. Shell injection is not possible because no free-form string reaches
these exec calls.

| File | Binary | Purpose |
|---|---|---|
| `src/shell.rs:107,135` | User's configured login shell | Resolve env vars at startup |
| `src/shell.rs:289` | `/usr/sbin/lsof` | Check port occupancy |
| `src/install.rs:77,93,97,112,123,562,610,626` | `git` | App install / update lifecycle |
| `src/cli.rs:1598,1627` | `unzip`, `cp` | App bundle extraction |
| `src/cli.rs:1685,1701` | `nohup`, `osascript` | Self-relaunch after update |
| `src/app_render.rs:35` | Resolved Python binary | Spawn the app subprocess |
| `src/process_app/mod.rs:355,359` | Python binary or explicit bin_path | Spawn the app subprocess |
| `src/cli_help_parser.rs:52` | CLI binary being probed | Harvest `--help` output for completions |
| `src/cli_crawl.rs:45,60` | CLI binary | Crawl subcommands for completion generation |
| `src/pane_ops/create.rs:588` | `plexi` / channel binary | Probe CLI for workspace path resolution |
| `src/app/canvas_bindings.rs:333,355` | `open` / `xdg-open` | Open URLs from canvas taps |
| `src/cli.rs:2061,2070` | `jq` | Pretty-print JSON output |
| `src/cli.rs:2571,2576` | `stty` | Terminal echo control during secret input |
| `src/cli.rs:2623,2631,2638` | Probed tool binary | Check tool availability for doctor command |
| `src/cli.rs:2984` | Configured CLI tool | Run a user-configured doctor check (internal constant list) |
| `src/cli.rs:3342` | `python3` | Check Python version during app scaffolding |

None of these accept app-supplied strings at the exec boundary. The set of binaries
is fixed at compile time or resolved from user config (shell path, Python path);
arguments are constructed by the host, not received over protocol.

---

## Invariants

1. **No new app-reachable `sh -c` path may be added without a capability gate** and
   a denial test in `src/testing.rs` following the pattern of
   `stream_process_denied_without_terminal_bindings`.

2. **No new `{token}` substitution** in a quick-note or similar user-config command
   template may be introduced without routing the substituted value through
   `shell_quote`. The template itself is trusted; the substituted values are not.

3. **`.plexi/commands.toml` is workspace-scoped executable code.** It is read from
   the user's project directory, not from a shared or system path. An app cannot
   read, modify, or invoke entries in this file; it is not part of the protocol.

4. **Secret values are never interpolated into command strings.** The `plexi run`
   path injects secrets exclusively as environment variables. If a command needs a
   secret value in a flag (e.g. `--token $MY_SECRET`), the command string
   references `$MY_SECRET` (a known env var name), not the secret value itself.

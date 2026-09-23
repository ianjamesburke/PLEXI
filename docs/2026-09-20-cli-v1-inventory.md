# CLI v1 Tree-Shake Inventory

**Status:** active
**Evidence date:** 2026-09-20; reconciled to `origin/alpha` on 2026-09-22

## Current v1 tags (G-13 reconciliation)

This section is the current tag inventory. It supersedes the historical
2026-09-20 findings and tag column below, which are retained as the original
tree-shake evidence and issue record. The source of truth for release tier is
`ReleaseFeature` in `src/release.rs`; the generated website reference is the
same clap command tree and is checked by `just check-cli-docs`.

| Surface | Current tag | Stable-v1 boundary |
|---|---|---|
| `run`, `workspace`, `secret` | v1 | Workspace and secret basics. |
| `agent report`, `agent status`, `agent hook` | v1 | Status hooks only; this does not promote the Assistant or agent mesh. |
| `context`, `pane`, `host`, `notify`, `note`, `notes`, `config`, `completions`, `demo`, `update`, `uninstall` | v1 | Terminal-host control and observation surface. `demo` refuses on platforms without a supported keymap. |
| `app` local-runtime commands | v1 | Local/remote/pack install, list/info/prune/freeze, open, uninstall, render, check/validate/inspect, state, init/test/package, action/update, plus `--cli`, `--mcp`, `cli:`, and `mcp:` wrappers. |
| `registry watch`, `descriptor probe`, `doctor`, `ai` diagnostics/setup | v1 | App-runtime and local CLI support surfaces; they are not the hosted Assistant product. |
| `events list`, `declare`, `emit`, `subscribe` | v1 | Brokered app-event surface. |
| `routine` and every routine subcommand | beta | `ReleaseFeature::Routines`; hidden from stable help and refused on stable execution. |
| `app publish`, `app browse`, `app search`, and bare marketplace-ID `app install` | beta | `ReleaseFeature::Marketplace`; local paths, remotes, and packs remain v1. |
| `account` and every account subcommand | beta | `ReleaseFeature::Marketplace`. |
| `events mcp-config` | beta | `ReleaseFeature::McpClient`; stable never emits an MCP endpoint or token. |
| `pane set-title`, `update apps` | removed | Former deceptive compatibility aliases; not part of the current command tree. |

There are no current `dead` entries: the five historical dead/deceptive
entries were either repaired or removed. This table deliberately keeps local
app runtime and SDK authoring in v1 while excluding marketplace, MCP client,
routines, and the full Assistant, matching the website CLI reference.

## Historical 2026-09-20 tree-shake evidence

Every `plexi` command and subcommand, tagged against Ian's v1 cut, with the
command run that produced each verdict. Nothing below is reasoned from source
alone: every behavioural claim names the invocation, its exit code, and its
output.

## Scope of the v1 cut used for tagging

v1 is the terminal host, the CLI, subcontexts, agent **status** hooks, and
Quick Note, on mac/linux/windows. Not v1: the full Assistant, the app/SDK
platform, the marketplace, DAW, media, and the browser surface.

- **`v1`** — needed for the terminal host, subcontexts, agent status, Quick
  Note, or workspace/pane/host basics.
- **`beta`** — release-gated already, or plainly Assistant / apps / SDK /
  marketplace / MCP / DAW / media.
- **`dead`** — broken, a stub that lies, unreachable, or `--help` promises
  something the command gets wrong on stable.

## How the evidence was produced

| | |
|---|---|
| Surface enumerated from | `src/cli/args.rs` (the clap tree), cross-checked against `plexi completions bash` and a recursive `--help` walk |
| Binary driven | the installed **stable** channel binary, `plexi 0.3.0` → `~/.local/share/plexi/bin/plexi`, profile `~/.plexi` |
| Channel comparison | the same binary copied to the basename `plexi-alpha` (`build_channel()` resolves the channel from the executable basename, so one build serves both tiers) |
| Host | a real headed host on `DISPLAY=:1` (Xvfb), started with `plexi host start`, driven through `PLEXI_SOCKET` and from inside a live pane |
| Passes | host running · host running with pane-scoped commands executed **inside** a real pane · host stopped |

**Provenance caveat, stated up front.** The installed stable binary is built
from `feature/linux-alpha-bringup`, not from `alpha` — `alpha` does not compile
on Linux at all (see [Alpha does not build on Linux](#alpha-does-not-build-on-linux)).
That branch contains `origin/alpha`, and its `src/cli/args.rs` and
`src/release.rs` are **byte-identical** to alpha's, so the enumerated surface
and every gating verdict below are alpha's. Only six files under `src/cli/`
differ (`ai.rs`, `doctor.rs`, `host.rs`, `mod.rs`, `run.rs`, `workspace.rs`);
where a behavioural finding touches one of those, it says so.

## Headline findings

### 1. `plexi --help` hides `host` and `events` entirely

`print_grouped_help` in `src/cli/help.rs` renders only the command names listed
in its `HELP_GROUPS` table. `Host` and `Events` are ordinary, non-hidden clap
subcommands (`src/cli/args.rs` marks only `Descriptor`, `_complete-open`,
`_complete-run` and the deprecated `pane set-title` with `hide = true`), but
neither name appears in `HELP_GROUPS`, so neither is printed.

```
$ plexi --help | grep -cE '^  (host|events) '
0
$ plexi host status
Plexi host is running (pid 1585814), 1 pane(s), socket /home/box/.plexi/notify.sock
```

`plexi host start` is how a user launches the product. On stable it is
undiscoverable from the CLI's own help while being fully documented on the
public website (`website/src/content/docs/cli.md` has a `## plexi host`
section, generated from the same clap tree).

### 2. Stable `--help` advertises the whole marketplace surface

The Marketplace release gate reaches exactly one help surface: the `account`
line in the top-level group list. Diffing the two channels' top-level help
shows that one line and nothing else:

```
$ diff <(plexi --help) <(plexi-alpha --help)
21a22
>   account       Manage your Plexi marketplace account (only needed to publish or buy paid apps)
```

`plexi app --help` is byte-identical across channels apart from the binary name
in the usage line, so stable lists `publish`, `browse` and `search` as if they
were available. They are not — the gate fires only at execution:

```
$ plexi app browse
error: marketplace requires the beta channel and is not part of the stable v1 surface. Use plexi-beta or plexi-alpha to try it.   (exit 1)
$ plexi app search notes      → same message, exit 1
$ plexi app publish .         → same message, exit 1
$ plexi account status        → same message, exit 1
$ plexi account logout        → same message, exit 1
```

`plexi account --help` and `plexi account login --help` also render full help on
stable with no mention of the gate, even though the parent is filtered out of
the group list.

### 3. `events mcp-config` bypasses the `McpClient` gate

`ReleaseFeature::McpClient` has a Beta minimum tier, and `src/cli/app_check.rs`
honours it. `plexi events mcp-config` does not — on stable, run inside a pane,
it hands out a live host MCP endpoint and bearer token:

```
$ plexi events mcp-config          # stable channel, inside pane 5
{
  "mcpServers": {
    "plexi-host": {
      "headers": { "Authorization": "Bearer 8c0f1801-…" },
      "type": "http",
      "url": "http://127.0.0.1:35381/mcp"
    }
  }
}                                                            (exit 0)
```

Every other MCP-client path is beta-only. This one is the config that turns the
capability on for an external agent, and it is ungated.

### 4. `context current` reports the wrong context — the subcontexts feature's own read path

`context current`'s help says it prints "the id and name of the current pane's
context". It prints whatever `PLEXI_CONTEXT_ID` was stamped into the pane's
environment at spawn, and that value is wrong both at spawn and after any
context move. Run inside pane 5, after `plexi context push`:

```
$ echo "ENV_CTX=$PLEXI_CONTEXT_ID"; plexi context current; plexi pane info | grep context_id
ENV_CTX=1
{
  "context_description": "cli inventory probe",
  "context_id": 1,
  "context_name": "Default"
}
  "context_id": 6,
```

`pane info` (host-resolved) says context 6; `context current` (env-resolved)
says context 1. `plexi pane list` agrees with `pane info`. The disagreement is
present at spawn too: `plexi pane new` placed pane 5 in a fresh context while
stamping the *caller's* context id into its environment.

This is the same class as the `PLEXI_CONTEXT_ROOT` staleness already documented
in `src/cli/AGENTS.md`, on a different variable, and it lands on the read path
of a named v1 feature.

### 5. `context zoom <nonexistent-id>` silently succeeds

```
$ plexi context zoom 999999     (exit 0, no output)
$ plexi context zoom 424242     (exit 0, no output)
```

`context list` confirms no such context exists. A bad id is indistinguishable
from a successful zoom.

### 6. `secret delete <nonexistent>` reports a deletion that never happened

```
$ plexi secret get NO_SUCH_SECRET
… not found …                                                (exit 1)
$ plexi secret delete GHOST
Deleted 'GHOST' from workspace 2f0f947a-747b-473f-89e2-184fbc1c7c9b   (exit 0)
$ plexi secret delete GHOST
Deleted 'GHOST' from workspace 2f0f947a-747b-473f-89e2-184fbc1c7c9b   (exit 0)
```

`get` knows the secret is absent; `delete` claims success twice for a name that
never existed. A user who typos a secret name is told the wrong key was removed.

### 7. `events declare` blocks forever on a GUI consent prompt with no CLI signal

`events declare --help` mentions that "the first declare under a namespace you
do not own prompts for host consent". The CLI prints nothing about that, offers
no timeout flag, and never returns:

```
$ timeout 25 plexi events declare inv.app inv.stream
(exit 124 — killed by timeout; zero bytes on stdout and stderr)
```

`plexi host screenshot` shows why: an **Event subscription request** modal is
waiting in the host UI. The modal is dismissible (clicking *Deny* cleared it),
so the host is healthy — but a headless or agent-driven caller sees an
indefinite hang with no diagnostic. Note that `plexi pane key <id> Escape`
cannot clear it: `pane key` targets the pane's PTY, not host-level overlays.

### 8. Host commands refuse to work outside a pane even when the host is running

With a live host on this channel and `PLEXI_SOCKET` unset:

```
$ plexi host status
Plexi host is running (pid 1585814), 9 pane(s), socket /home/box/.plexi/notify.sock   (exit 0)
$ plexi pane list
error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane                (exit 1)
$ plexi context list   → same
$ plexi notify …       → same
$ plexi events list    → same
$ plexi agent status   → same
$ plexi pane list --socket /home/box/.plexi/notify.sock
[ { "agent": { … } … ]                                                                (exit 0)
```

`host status` discovers and prints the exact socket path that the very next
command claims not to know. `--socket` is the working escape hatch and the
error message never names it.

### 9. Output-stream discipline is inconsistent

Success output lands on stderr in several places, which breaks piping:

```
$ plexi secret list 1>/dev/null
No secrets stored.
$ plexi config check 1>/dev/null
✓ /home/box/.plexi/config.toml is valid
✓ /tmp/plexi-inv/ws/.plexi/config.toml is valid
$ plexi pane slot write invslot hello 1>/dev/null
slot "invslot" <- 5 bytes
```

Raw `fern` log records also bleed onto CLI stderr instead of a clean `error:`
line — in alpha code paths:

```
$ plexi pane click 1 10 10
[2026-09-20 03:25:59] [WARN] [plexi::cli::pane] pane_click:cli: host reported error: pane …
$ plexi descriptor probe echo
[2026-09-20 03:26:28] [WARN] [plexi::cli::descriptor] cli_resolve: crawl failed for `echo` …
$ plexi secret get NO_SUCH_SECRET
[2026-09-20 03:25:56] [WARN] [plexi::cli::workspace] secret_get:cli: not found friendly=NO…
```

### 10. `config set` and `config reset`/`config get` default to different scopes

Inside a workspace, `config set` writes the *workspace* file (and silently
materialises a full default config there as a side effect), while `config get`
and `config reset` act on the *global* file:

```
$ plexi config get log.level
error: config key "log.level" not set (no value at "log" in the current config)   (exit 1)
$ plexi config set log.level=info
✓ wrote default config to /tmp/plexi-inv/ws/.plexi/config.toml
✓ wrote /tmp/plexi-inv/ws/.plexi/config.toml
$ plexi config reset
backed up existing config to /home/box/.plexi/config.toml.bak
✓ wrote default config to /home/box/.plexi/config.toml
```

### 11. macOS-only strings ship on the Linux build

v1 targets mac/linux/windows. On Linux:

```
$ plexi notes open
error: fzf is not installed — run `brew install fzf` to enable the picker      (exit 1)
$ plexi ai doctor
  ✗ Ollama not installed  (brew install ollama)
tip: … or `brew install ollama`
$ plexi demo            # inside a real pane
  Step 1 of 11   Split right
    Press Command-D. Plexi splits this pane to the right …
    Press [ ⌘D ] to split right.
(exit 124 — never advances, never returns)
```

`plexi demo` is the CLI's own first-run teaching surface and it teaches a
keystroke that does not exist on the platform it is running on.

### 12. Help text leaks internal ticket ids and SDK authoring docs onto stable

```
$ plexi app --help | grep state
  state      Read or replace a file-backed app's state document (stint 0645)
```

`src/cli/args.rs` carries `(stint 0645)` in a user-facing doc comment.
Separately, `plexi app init --help` prints a multi-screen `APP DEVELOPMENT
GUIDE` — SDK authoring content — on the stable channel where the app/SDK
platform is explicitly not v1.

### 13. Two aliases are pure duplication

`plexi update apps` is documented as a "Compatibility alias for `plexi app
update`" and both produce identical output; `plexi pane set-title` is
`hide = true` and its own help says "Deprecated: use `plexi pane name`
instead".

### Alpha does not build on Linux

`just build` on `alpha` at `15d9a874` fails on Linux with two hard compile
errors plus eleven `-D warnings` failures:

- `src/app/mod.rs` calls `crate::platform::macos_menu::apply_version_title_once()`
  and `take_reload_config_flag()` with no `#[cfg(target_os = "macos")]` guard;
  the module is macOS-gated in `src/platform/mod.rs` (`E0433`, twice).
- `src/app/mod.rs` calls `UnixStream::peer_cred()`, an unstable library feature
  (`peer_credentials_unix_socket`, `E0658`, twice).
- Nine unused-import / unused-variable / unnecessary-`mut` errors in
  `src/app/secrets_app.rs`, `src/host/shell.rs`, `src/plexi_ai/broker.rs` and
  the secrets and route modules, all Linux-only dead code under
  `RUSTFLAGS="-D warnings"`.

`feature/linux-alpha-bringup` carries the fixes; until it lands, the pre-push
gate in `src/testing/TESTING.md` cannot be satisfied on a Linux checkout.

## Verified-good (no action)

Recorded because they were driven and passed, not assumed:

- `context sub` answers in one round trip, as `src/cli/AGENTS.md` requires:
  `{"context_id":8,"panes":[8],"windows":[{"grid_x":0,"grid_y":0,"window_id":9}]}`
- `notify dismiss` correctly refuses an unresolvable caller (`error: notify
  dismiss requires a resolvable caller pane`, exit 1) out of pane, and succeeds
  (exit 0) from inside the pane that posted the notification. The host resolves
  the sender from socket-peer ancestry, not from client-claimed ids.
- `pane wait --until idle --timeout 3` returns exit **2** on timeout, the
  documented distinct code.
- `pane slot` write/read/list/delete round-trips; a deleted slot read returns
  `error: slot 'invslot' not found`, exit 1.
- `plexi note` resolves its tier from cwd, not `PLEXI_CONTEXT_ROOT`: from an
  anchored workspace it wrote `/tmp/plexi-inv/ws/.plexi/notes/…`; from a pane
  whose cwd is unanchored it wrote `~/.plexi/notes/…`. `notes list` rolled up
  both tiers consistently.
- `host start` / `stop` / `status` / `log` / `screenshot` all work headed on
  Xvfb; `host screenshot` produced a real 1280x719 PNG through the render
  pipeline.
- With the host stopped, every host-dependent command fails fast with exit 1.
  Nothing hung.
- `app render <path>` renders headlessly without a host.

## The matrix

Legend: **v1** / **beta** / **dead**. "Finding" points at the numbered headline
above.

### Workspace

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `run` | v1 | exit 0, lists workspace commands | |
| `run <unknown>` | v1 | exit 1, `error: unknown command` | |
| `workspace init` | v1 | exit 0, created `.plexi/` | |
| `workspace clean` | v1 | exit 0 with host; exit 1 without | |
| `secret set` | v1 | help only (keychain write) | |
| `secret get` | v1 | exit 1 on missing | 9 |
| `secret list` | v1 | exit 0 | 9 |
| `secret delete` | **dead** | exit 0 + false "Deleted" for absent key | **6** |
| `routine list` | v1 | exit 0 | |
| `routine add` | v1 | exit 0, wrote `routines.toml` | |
| `routine run` | v1 | exit 0, printed spawned pane id | |
| `routine remove` | v1 | exit 0 | |
| `routine enable` | v1 | exit 0 | |
| `routine disable` | v1 | exit 0 | |
| `routine run <unknown>` | v1 | exit 1, clean error | |

### Agent

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `agent report` | v1 | exit 0; state visible in `agent status`/`pane list` | |
| `agent status` | v1 | exit 0, table | |
| `agent hook install` | v1 | help only (mutates agent configs) | |
| `agent hook uninstall` | v1 | help only | |
| `agent init` | beta | scaffolds an app with `ai.query` + chat UI → SDK/Assistant | |
| `agent add` | beta | exit 1, clean error | |
| `agent update` | beta | exit 1, clean error | |
| `agent list` | beta | exit 0 | |

### Context (v1 feature)

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `context list` | v1 | exit 0, JSON | |
| `context new` | v1 | exit 0, context created | |
| `context sub` | v1 | exit 0, ids returned in one round trip | |
| `context push` | v1 | exit 0, pane moved | |
| `context zoom-out` | v1 | exit 0 | |
| `context describe` | v1 | exit 0, visible in `context list` | |
| `context set-root` | v1 | exit 0 | |
| `context open` | v1 | help only | |
| `context current` | **dead** | reports the wrong context id | **4** |
| `context zoom` | **dead** | exit 0 for a nonexistent id | **5** |

### Pane

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `pane list` / `self` / `info` | v1 | exit 0 | 4 |
| `pane new` | v1 | exit 0, returns id | 4 |
| `pane name` | v1 | exit 0 | |
| `pane set-title` | **dead** | hidden, self-described deprecated alias | **13** |
| `pane close` | v1 | driven on a scratch pane | |
| `pane focus` | v1 | exit 0 | |
| `pane send` | v1 | exit 0, text appeared in pane | |
| `pane command` | v1 | exit 0; ran in pane | |
| `pane key` | v1 | exit 0; PTY-scoped, not overlay-scoped | 7 |
| `pane capture` | v1 | exit 0, JSON + `--plain` | |
| `pane state` | v1 | exit 0; exit 1 + clean error for bad id | |
| `pane status` | v1 | exit 0, composite verdict | |
| `pane wait` | v1 | exit 2 on timeout, as contracted | |
| `pane events --follow` | v1 | streams; no timeout flag by design | |
| `pane heartbeat` | v1 | exit 0, `{"heartbeat":null,"ok":true}` | |
| `pane drop` | v1 | exit 1 on a terminal pane, clean error | |
| `pane slot *` | v1 | full round-trip verified | 9 |
| `pane click` | beta | app-pane only → app platform | 9 |
| `pane drag` | beta | app-pane only → app platform | |

### Host (v1 feature)

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `host start` | v1 | launched a real host on `DISPLAY=:1` | **1** |
| `host stop` | v1 | `Plexi host stopped (clean shutdown).` | 1 |
| `host status` | v1 | exit 0 running *and* not-running | 1, 8 |
| `host log` | v1 | exit 0 | 1 |
| `host screenshot` | v1 | exit 0, real PNG | 1 |

### Notify / Notes / Config / System

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `notify` | v1 | exit 0, returned notify id, modal rendered | |
| `notify dismiss` | v1 | exit 0 in-pane; exit 1 out of pane by design | |
| `note` | v1 | exit 0, correct tier | |
| `notes list` | v1 | exit 0, rolled up tiers | |
| `notes open` | v1 | exit 1, `brew install fzf` on Linux | **11** |
| `config check` | v1 | exit 0 | 9 |
| `config list` / `get` / `edit` | v1 | exit 0 / exit 1 on unknown key | 10 |
| `config set` | v1 | exit 0, workspace scope | **10** |
| `config reset` | v1 | exit 0, global scope | **10** |
| `completions` | v1 | bash/fish emitted; exit 1 on unknown shell | |
| `demo` | v1 | exit 124 in-pane, teaches ⌘D on Linux | **11** |
| `update` | v1 | help only (self-update) | |
| `update apps` | **dead** | self-described compatibility alias | **13** |
| `uninstall` | v1 | help only | |
| `_complete-open` / `_complete-run` | v1 | hidden completion helpers | |

### Apps / SDK / marketplace (beta)

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `app list` / `info` / `prune` / `freeze` | beta | driven, exit 0 / clean exit 1 | |
| `app open` / `uninstall` / `install` / `trust` | beta | clean exit 1 on bad input | |
| `app render` | beta | exit 0, JSON frame tree | |
| `app check` / `validate` / `inspect` | beta | clean exit 1 on a non-app dir | |
| `app state` / `state get` / `state set` | beta | clean exit 1 for a stateless app | **12** |
| `app action` | beta | exit 1, `pane 1 is not an app pane` | |
| `app update` | beta | exit 0 | |
| `app init` / `test` / `package` | beta | help only | **12** |
| `app publish` / `browse` / `search` | beta | gate fires at execution, listed in help | **2** |
| `registry watch` | beta | exit 0, `[STALE] git …` | |
| `descriptor probe` | beta | exit 1, hidden already | 9 |
| `doctor` | beta | exit 0; audits installed apps | |

### AI / Assistant (beta)

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `ai doctor` | beta | exit 0 | 11 |
| `ai onboard` | beta | exit 0 | 11 |
| `ai setup` | beta | help only (interactive wizard) | |

### Events (beta)

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `events list` | beta | exit 0 | |
| `events declare` | beta | exit 124 — blocks on a GUI consent modal | **7** |
| `events emit` | beta | exit 1, clean error | |
| `events subscribe` | beta | help only (long-lived stream) | |
| `events mcp-config` | beta | exit 0 on **stable** — gate bypassed | **3** |

### Account / marketplace (beta, gated)

| Command | Tag | Driven | Finding |
|---|---|---|---|
| `account status` | beta | gate message, exit 1 | 2 |
| `account logout` | beta | gate message, exit 1 | 2 |
| `account login` | beta | help only | 2 |

## Reproducing

```bash
export DISPLAY=:1
plexi host start
export PLEXI_SOCKET=$HOME/.plexi/notify.sock
# out-of-pane commands
plexi pane list --socket "$PLEXI_SOCKET"
# pane-scoped commands must run inside a real pane, not with a faked
# PLEXI_PANE_ID: `notify dismiss` resolves its sender from socket-peer
# process ancestry.
ID=$(plexi pane new); plexi pane command "$ID" "<your probe script>" --enter
plexi host stop
```

Channel comparison needs no second build — copy the binary to the basename
`plexi-alpha` and run it; `build_channel()` reads the executable basename.

## Issues filed

| # | Finding | Title | Priority |
|---|---|---|---|
| [2619](https://github.com/ianjamesburke/PLEXI/issues/2619) | 1 | `plexi --help` never lists `host` or `events` | P1 |
| [2620](https://github.com/ianjamesburke/PLEXI/issues/2620) | 2 | stable `--help` advertises marketplace commands the gate then refuses | P1 |
| [2621](https://github.com/ianjamesburke/PLEXI/issues/2621) | 3 | `events mcp-config` bypasses the McpClient beta gate | P1 |
| [2622](https://github.com/ianjamesburke/PLEXI/issues/2622) | 4, 5 | `context current` reports the wrong context; `context zoom` accepts a nonexistent id | P1 |
| [2623](https://github.com/ianjamesburke/PLEXI/issues/2623) | 6 | `secret delete` reports success for a key that does not exist | P2 |
| [2624](https://github.com/ianjamesburke/PLEXI/issues/2624) | 7 | `events declare` blocks forever on a GUI consent prompt | P2 |
| [2625](https://github.com/ianjamesburke/PLEXI/issues/2625) | 8 | host commands refuse to run outside a pane when the host is running | P2 |
| [2626](https://github.com/ianjamesburke/PLEXI/issues/2626) | 9 | inconsistent stdout/stderr discipline | P3 |
| [2627](https://github.com/ianjamesburke/PLEXI/issues/2627) | 10 | `config set` and `config get`/`reset` default to different scopes | P2 |
| [2628](https://github.com/ianjamesburke/PLEXI/issues/2628) | 11 | macOS-only strings ship on the Linux build | P2 |
| [2629](https://github.com/ianjamesburke/PLEXI/issues/2629) | 12, 13 | help-surface cleanup: stint id, SDK guide on stable, two dead aliases | P3 |
| [2630](https://github.com/ianjamesburke/PLEXI/issues/2630) | — | alpha does not compile on Linux | P1 |

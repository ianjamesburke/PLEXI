# CLI-Backed App Contract (`plexi app open --cli`)

Plexi can turn any command-line tool into a GUI without an SDK. You point it at a
binary, Plexi resolves a UI descriptor for that binary, and a native host app —
the **`cli-renderer`** — draws a navigable command list, per-command forms, and
runs the assembled command in a linked terminal pane.

This document is the **runtime contract** for those apps: how a CLI-backed pane
launches, becomes ready, runs commands, crashes, and is inspected. It does *not*
re-document the descriptor schema — for that, read the
[CLI Descriptor Authoring Guide](../../registry/CLI_DESCRIPTOR_GUIDE.md) and the canonical
schema at `schemas/plexi-descriptor-schema.json`.

- **What the descriptor looks like** → [`CLI_DESCRIPTOR_GUIDE.md`](../../registry/CLI_DESCRIPTOR_GUIDE.md)
- **The host/app wire protocol for SDK apps** → `src/protocol/` and [`SDK_V2.md`](../../sdk/python/SDK_V2.md)
- **The capability/permission model** → [`SECURITY_MODEL.md`](../process_app/SECURITY_MODEL.md)

The renderer is implemented in `src/render/cli_renderer_app.rs`. The open path is
`src/cli/open.rs` and `src/cli/descriptor.rs`. The linked-terminal dispatch is
`src/app/canvas_bindings.rs`.

---

## 1. What a CLI-backed app is (and is not)

A CLI-backed app is **not** a PGAP subprocess. There is no Python, no manifest,
no `Init` handshake, and no per-app capability set. It is a single compiled-in
host app — `CliRendererApp`, registered as the builtin id `"cli-renderer"` in
`builtin_factory` (`src/pane_ops/create.rs:40`) — parameterized by one argument:
the path to a resolved descriptor JSON file.

Because it is a native builtin, the renderer runs on the host UI thread with the
host's privileges. It is a *UI generator over a descriptor*, not a sandbox. The
isolation boundary is the **linked terminal**: the renderer never executes
anything itself — it writes the assembled command string into a real terminal
PTY, and your shell runs it exactly as if you had typed it. See
[§5 Permissions](#5-permissions-and-the-trust-boundary) for what this means.

Two distinct things share the word "CLI":

| Concept | Where it lives | Lifetime |
|---|---|---|
| **Descriptor resolution** (Tier 1/2/3) | `src/cli/descriptor.rs`, run by the `plexi` CLI process at open time | One-shot, before the pane exists |
| **The renderer pane** (`cli-renderer`) | `src/render/cli_renderer_app.rs`, a host app | Lives as long as the pane is open |

The descriptor is resolved *once*, frozen to a temp file, and handed to the
renderer. The renderer never re-resolves. This is load-bearing for
[§4 Descriptor caching and invalidation](#4-descriptor-caching-and-invalidation).

---

## 2. Launch sequence

### 2.1 From CLI to pane

`plexi app open --cli <binary>` (or the `cli:<binary>` prefix) routes through
`open_cli_by_name` (`src/cli/open.rs:269`):

1. **Resolve** the descriptor through the full tier chain —
   `descriptor::resolve_cli` (`src/cli/descriptor.rs:131`): Tier 1 native
   `<binary> --plexi`, Tier 2 registry, Tier 3 recursive `--help` crawl.
2. **Freeze** the resolved descriptor to a temp file —
   `open_descriptor_in_renderer` (`src/cli/open.rs:230`) writes
   `serde_json::to_string_pretty(descriptor)` to
   `$TMPDIR/plexi-descriptor-<uuid>.json`.
3. **Spawn** a pane running the `cli-renderer` builtin with that temp path as its
   sole launch arg (`pane_new_cli(..., Some("cli-renderer"), &[], &[path])`).

If resolution fails, the CLI prints `error: could not resolve CLI ...` and exits
non-zero — no pane is created. If the temp write fails, it prints
`error: could not write descriptor temp file` and exits non-zero.

### 2.2 Inside the renderer: load

`CliRendererApp::new(descriptor_path)` (`src/render/cli_renderer_app.rs:85`) is
the only constructor. On construction it reads and parses the descriptor file
and sets the initial `View`:

| Outcome | View | Logged |
|---|---|---|
| File read + JSON parse OK | `View::Loading` | `CliRendererApp: loaded descriptor for '<name>' v<version>` (info) |
| JSON invalid | `View::Error("Invalid descriptor JSON: …")` | `bad JSON in <path>` (warn) |
| File unreadable | `View::Error("Cannot read <path>: …")` | `cannot read <path>` (warn) |

A descriptor that fails to load **does not crash the pane** — it renders the
`Error` view with the message and stays alive. This is verified by
`bad_descriptor_path_yields_error_view` (`src/render/cli_renderer_app.rs:1046`).

---

## 3. Ready, run, and reload

### 3.1 Ready (linked terminal handshake)

The renderer cannot run anything until it has a terminal to run it in. On the
**first render frame** (`App::ui`, `src/render/cli_renderer_app.rs:739`), if a
descriptor loaded successfully, the renderer:

1. Calls `request_terminal` (`:292`), which enqueues an
   `AppCommand::RequestLinkedTerminal` with `place_below: true` (so the output
   terminal stacks *under* the form, not beside it — verified by
   `requests_linked_terminal_below`, `:1029`).
2. Transitions `View::Loading → View::List`.

The host drains that command in `PlexiApp` (`src/app/mod.rs:2378`), which calls
`dispatch_request_linked_terminal` (`src/app/canvas_bindings.rs:26`). That
handler splits a new `TerminalPane` adjacent to the renderer, sets the
renderer's `linked_pane_id`, and emits `PlexiEvent::LinkedTerminalReady { request_id,
terminal_pane_id }` back to the renderer.

The renderer receives that event in `queue_outbound_event` (`:801`). If the
`request_id` matches its own, it stores `terminal_pane_id` and logs
`CliRendererApp: linked terminal ready, pane_id=<id>`. **Until this fires,
`terminal_pane_id == 0`** and the Run button shows
`"▶  Run  (connecting terminal…)"` (`:540`).

> **Ready state is observable, not promised.** If the sender's tile is gone, or
> the `TerminalPane` fails to construct, the host emits
> `LinkedTerminalReady { terminal_pane_id: 0 }` (`canvas_bindings.rs:55`, `:90`)
> so the renderer unblocks instead of hanging. In that degraded state the
> renderer stays usable for browsing but `execute()` is a no-op (see §3.2).

### 3.2 Run

A command runs when the user presses **Enter** in a leaf form (`handle_key`,
`:729`) or clicks the Run button (`:545`). Both call `execute` (`:273`):

- If `terminal_pane_id == 0`, `execute` logs `no linked terminal, cannot run`
  and returns — **nothing is sent**.
- Otherwise it assembles the command string with `build_command_string` (`:228`)
  — binary name + breadcrumb path + flags + positional args, with
  space-containing values single-quoted and bool flags rendered bare — and
  enqueues `AppCommand::RunInLinkedTerminal { terminal_pane_id, command, echo: true }`.

The host drains it (`src/app/mod.rs:2392`) into
`dispatch_run_in_linked_terminal` (`src/app/canvas_bindings.rs:147`), which
**re-validates** that `terminal_pane_id` still matches the renderer's
`linked_pane_id` before writing `"<command>\n"` into the terminal's PTY. A
mismatch is rejected with a warn and dropped — the renderer cannot drive an
arbitrary terminal, only the one it was linked to.

`Cmd+Enter` is deliberately **not** consumed — it passes through to the host
pane-zoom toggle (`:710`, `:729`; verified by
`cmd_enter_passes_through_for_pane_zoom`, `:995`).

> **`echo` is observational.** The `echo: true` field is preserved on the wire
> but PTY-level echo is shell-controlled; the host does not suppress or force it
> (`canvas_bindings.rs:141` doc comment).

### 3.3 Reload — there is none (de facto)

The renderer does **not** watch, re-read, or hot-reload its descriptor. The temp
file written at open time is a frozen snapshot; `CliRendererApp` reads it once in
`new()` and never touches it again. `serialize_state` returns `None` and
`restore_state` is a no-op (`:818`–`:822`), so a CLI-backed pane has **no
persisted state** and does not survive a layout restore the way a stateful PGAP
app does.

To pick up a changed descriptor (e.g. the CLI shipped a new version, or you
edited a registry file), **close the pane and re-open it** — re-resolution
happens entirely in the CLI process before the new pane is created. See §4 for
where the durable cache (and its invalidation) actually lives.

---

## 4. Descriptor caching and invalidation

There are two caches in this system, at two layers. The renderer pane itself
holds neither — it only ever sees a finished temp file.

### 4.1 The temp snapshot (per-open, ephemeral)

Every `plexi app open --cli` writes a fresh
`$TMPDIR/plexi-descriptor-<uuid>.json` (`src/cli/open.rs:245`). It is unique per
open (UUID-named), never reused, and never invalidated — a new one is written on
the next open. It is the immutable input to one renderer instance.

### 4.2 The crawl cache (durable, version-keyed)

Tier 3 (`--help` crawl) is the only resolution tier with a durable cache, in
`src/cli/crawl.rs`. It writes the inferred descriptor to:

```text
<config_dir>/cache/descriptors/<cli>.json
```

where `<config_dir>` is the channel-scoped profile dir (`crate::config::config_dir()`,
`crawl.rs:118`) — `~/.plexi/`, `~/.plexi-alpha/`, `~/.plexi-pr-<N>/`, etc. See §6.

**Invalidation is by CLI version** (`crawl_with_runner`, `crawl.rs:143`):

1. On a cache hit, the crawler reads `<cli> --version` and compares it to the
   `version` field stored in the cached descriptor.
2. If they differ, the cache is **stale** — it logs
   `cache stale for '<cli>' — version changed from <old> to <new>; re-crawling`
   and re-crawls from scratch.
3. If `--version` cannot be read, the cache is *not* invalidated (treated as
   non-stale) — a CLI with no version string keeps its cached descriptor.
4. A cached file that fails to deserialize falls through and is re-crawled.

Tier 1 (native `--plexi`) and Tier 2 (registry) are **not** crawl-cached — they
are re-resolved on every open. Tier 1 always reflects the CLI's current output;
Tier 2 reflects the registry file on disk. The crawl cache is the only place
where a *stale* descriptor can persist, and the version check is the only
invalidation mechanism. (Registry version pinning — `<version>.json` filenames —
is a separate authoring concern documented in `CLI_DESCRIPTOR_GUIDE.md` §6.)

> There is no TTL and no manual `--no-cache` open flag for the crawl cache; the
> only invalidation is the version delta above, or deleting the cache file. See
> [GAPS](#gaps-tracked-separately) — this is documented as-is, not endorsed.

---

## 5. Permissions and the trust boundary

**A CLI-backed app prompts for nothing.** Unlike a PGAP app — whose
`terminal.bindings`, `fs.read`, `net.http`, etc. capabilities are gated in
`src/process_app/routing.rs` and surfaced as grant modals (see
`src/app/permissions.rs`) — the `cli-renderer` is a native builtin and its
linked-terminal commands are dispatched **directly** by `PlexiApp` with no
capability check (`canvas_bindings.rs:26`, `:147`). There are zero permission or
`Capability` references in `cli_renderer_app.rs`.

This is by design and must be stated plainly to anyone authoring or auditing a
descriptor:

- **Command execution:** the renderer assembles a shell command and writes it to
  a real PTY. Your shell runs it with your full user privileges. There is **no
  prompt** before a command runs — pressing Enter in a form is equivalent to
  typing that command and hitting return in a terminal.
- **Filesystem and network:** the renderer performs neither directly. Whatever
  the *underlying CLI* does when you run it — read files, write files, make
  network calls — happens unmediated in the terminal. Plexi does not interpose.
- **Descriptor `writes` / `reads` fields:** the descriptor's `writes`/`reads`
  arrays (`CLI_DESCRIPTOR_GUIDE.md` §3) are **advisory metadata** for authoring
  and future trust gating. The shipped renderer does **not** consult them to
  prompt before a command runs. See [GAPS](#gaps-tracked-separately).

The practical contract: **a CLI-backed app is exactly as trusted as the binary
it wraps.** Resolving a descriptor (running `<cli> --plexi`, `--help`, or
`--version`) only invokes those introspection flags — never the command itself
(`crawl.rs` module doc, "only `--help` is ever invoked"). But once a form is
submitted, the wrapped CLI runs with no sandbox. Treat opening `--cli <binary>`
as equivalent to trusting `<binary>` on your shell.

---

## 6. Channel-agnostic behavior

A CLI-backed app behaves **identically** on `main`, `rc-*`, `beta`, `alpha`,
and `plexi-pr-<N>` builds. The channel is an implementation detail; nothing in
the contract above is channel-specific.

- **Descriptor resolution** is pure CLI-process logic and channel-independent at
  the algorithm level.
- **The crawl cache path** is derived from `crate::config::config_dir()`
  (`crawl.rs:118`), so each channel caches into its own profile dir
  (`~/.plexi/cache/...`, `~/.plexi-rc-010/cache/...`,
  `~/.plexi-alpha/cache/...`, `~/.plexi-pr-<N>/cache/...`).
  Caches never leak across channels.
- **The registry path** (Tier 2) is likewise `~/.plexi-<channel>/registry/...`
  (`CLI_DESCRIPTOR_GUIDE.md` §6).
- **The renderer** is compiled into every channel's binary; the same
  `cli-renderer` builtin id resolves on all of them.

Per the Channel-Agnostic CLI Rule (root `CLAUDE.md`): to target a specific
channel's running instance, invoke the full binary name
(`plexi-alpha app open --cli git`, `plexi-pr-817 app open --cli git`). The bare
`plexi` name routes to the `main` channel binary.

---

## 7. Logging and inspection

CLI-backed apps are instrumented at every state transition. Check the
channel-specific log first when debugging (`~/.plexi-<channel>/plexi.log`).

### 7.1 Host log traces

Grep `plexi.log` for these `info`-level lines (all in
`src/render/cli_renderer_app.rs` unless noted):

| Trace | When |
|---|---|
| `CliRendererApp: loaded descriptor for '<name>' v<version>` | Descriptor parsed on construction |
| `CliRendererApp: requested linked terminal` | First-frame terminal request |
| `CliRendererApp: linked terminal ready, pane_id=<id>` | `LinkedTerminalReady` received |
| `CliRendererApp: queued run '<cmd>'` | A command was assembled and queued |
| `CliRendererApp: auto-focused field '<name>'` | First text field grabbed focus |
| `RequestLinkedTerminal: pane <s> → terminal <t>` (`canvas_bindings.rs:128`) | Host created the linked terminal |
| `RunInLinkedTerminal: pane <t> ← "<cmd>\n"` (`canvas_bindings.rs:178`, debug) | Host wrote to the PTY |
| `open:cli: launching cli-renderer for '<name>' with descriptor at <path>` (`open.rs:252`) | The temp file path used for this open |

Warn-level lines flag the degraded paths: `no linked terminal, cannot run`
(`:275`), `bad JSON in <path>` (`:97`), `cannot read <path>` (`:102`),
`RunInLinkedTerminal: ... not linked` (`canvas_bindings.rs:161`).

### 7.2 Pane inspection

A CLI-backed pane is an **App pane**, so the standard inspection commands apply
(all over `PLEXI_SOCKET`, see `src/cli/pane.rs`):

```bash
plexi pane list            # the cli-renderer pane appears as an app pane
plexi pane info            # metadata for the current pane
plexi pane state <id>      # the renderer's current UI snapshot
```

On macOS, bare `plexi` is a contextual shim. Inside a Plexi PTY it delegates to
the binary named by `PLEXI_CHANNEL` when that channel binary is installed, so
the examples above inspect the current channel. Outside Plexi, bare `plexi`
falls back to the stable app-bundle binary.

- The pane's display name comes from `App::display_name`
  (`cli_renderer_app.rs:679`): the descriptor's `name`, or `"CLI Renderer"` if
  the descriptor failed to load.
- The pane's app type id is `"cli-renderer"` (`App::type_id`, `:675`).
- The linked terminal it spawns is a separate **Terminal** pane and lists
  independently; the two are associated by the renderer's `linked_pane_id`, not
  by a shared pane entry.

To drive a CLI-backed pane like any other app for testing, use the
render → inspect → act loop from
[`SDK_QUICKSTART.md`](../../sdk/python/SDK_QUICKSTART.md) §5: `plexi pane key <id> down`,
`plexi pane key <id> enter`, then `plexi pane state <id>` to confirm.

---

## 8. Lifecycle state summary

| State | Trigger | Renderer behavior |
|---|---|---|
| **Load** | `CliRendererApp::new(path)` | Read + parse descriptor; set `Loading` or `Error` |
| **Error** | Bad/missing descriptor | Render error message; pane stays alive; no terminal requested |
| **Loading → List** | First `ui()` frame, descriptor OK | Request linked terminal, show command list |
| **Ready** | `LinkedTerminalReady` event matches `request_id` | Store `terminal_pane_id`; Run button enabled |
| **Run** | Enter in a form / Run click, terminal ready | Write assembled command to the linked terminal PTY |
| **Run blocked** | Same, but `terminal_pane_id == 0` | No-op; warn `no linked terminal, cannot run` |
| **Reload** | (none) | No watch/reload; close and re-open to refresh |
| **Close** | Pane closed by user/host | `wants_close()` is always `false` — the renderer never self-closes; the host owns close |
| **Crash / restore** | — | `serialize_state()` is `None`; the pane is not restored with state on layout reload |

`wants_close` returning `false` (`:793`) means a CLI-backed pane is closed only
by an explicit host/user action (e.g. closing the pane), never by the app
deciding it is done.

---

## GAPS (tracked separately)

These are gaps between the shipped behavior and an ideal contract. They are
documented here as *current behavior*; the fixes are tracked as issues:
**#2244** (cache invalidation — gaps 1-3), **#2245** (command-execution trust —
gaps 4-5), **#2246** (degraded-ready + temp cleanup — gaps 6-7). This doc does
not fix them.

1. **No reload / re-resolution affordance.** `cli_renderer_app.rs:818-822`
   (`serialize_state`/`restore_state` are no-ops) and the fact that `new()` reads
   the descriptor once mean there is no way to refresh a descriptor without
   closing and re-opening the pane. A `watch`/reload binding or a
   `plexi app reload` for cli-renderer panes is absent.

2. **No manual crawl-cache invalidation.** `src/cli/crawl.rs:143` invalidates
   only on a CLI version delta. There is no TTL, no `--refresh`/`--no-cache`
   open flag, and no `plexi descriptor cache clear` command. A CLI whose
   `--version` is unchanged but whose `--help` surface changed will serve a stale
   crawled descriptor indefinitely.

3. **Version-less CLIs never invalidate.** `src/cli/crawl.rs:150-153`: when
   `<cli> --version` returns nothing, `stale` is `false`, so the crawl cache is
   pinned forever for that CLI. There is no content-hash fallback.

4. **`writes`/`reads` descriptor fields are inert.** The descriptor carries
   `writes`/`reads` arrays (documented in `CLI_DESCRIPTOR_GUIDE.md` §3 as "trust
   gating") but the renderer never consults them — `execute`
   (`cli_renderer_app.rs:273`) runs the command unconditionally. There is no
   confirm-before-write prompt. The fields are advisory metadata only.

5. **No command-execution confirmation.** `cli_renderer_app.rs:273` →
   `canvas_bindings.rs:147`: submitting a form writes straight to the PTY with no
   user confirmation, even for destructive commands. A CLI-backed app is exactly
   as trusted as the wrapped binary, with no per-run gate.

6. **Stale temp files accumulate.** `src/cli/open.rs:245` writes
   `plexi-descriptor-<uuid>.json` to `$TMPDIR` on every open and never deletes
   it. There is no cleanup on pane close. (Low priority — OS temp reaping
   eventually handles it — but it is unbounded growth between reboots.)

7. **Degraded-ready state is silent to the user.** When the host emits
   `LinkedTerminalReady { terminal_pane_id: 0 }` (`canvas_bindings.rs:55`, `:90`),
   the renderer keeps `terminal_pane_id == 0` and the Run button stays in its
   `(connecting terminal…)` label forever, with no surfaced error explaining why
   the terminal never linked. The only signal is a host-log warn line.

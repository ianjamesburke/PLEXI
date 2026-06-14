# Linked Terminal Bindings Contract

How a Plexi app drives a terminal. This is the developer-facing reference for the
`terminal.bindings` capability and the five operations it gates. It documents the
contract as implemented (stint 0062 hardened it; stint 0086 formalized it here),
including the permission-error contract, lifecycle, and known gaps.

## Capability

All five operations are gated by a single capability: **`terminal.bindings`**
(`Capability::TerminalBindings`). An app declares it in `manifest.toml`:

```toml
[app.capabilities]
capabilities = ["terminal.bindings"]
```

A request from an app without the grant is denied at the host routing boundary
(`src/process_app/routing.rs`) before any host effect. Denial surfaces depend on
the operation kind — see [Permission error contract](#permission-error-contract).

## The five operations

| Operation | Kind | Returns | SDK shape |
|---|---|---|---|
| `RequestLinkedTerminal` | request-response | a terminal pane id | awaits `LinkedTerminalReady` |
| `RequestCommandPreview` | request-response | the cwd a command would run in | awaits `CommandPreview` |
| `RunInLinkedTerminal` | fire-and-forget | nothing | no response |
| `InsertPathToken` | fire-and-forget | nothing | no response |
| `OpenArtifact` | fire-and-forget | nothing | no response |

- **`RequestLinkedTerminal`** — opens a terminal pane beside the app and records
  it as the app's linked terminal (`linked_pane_id` on the app pane). The host
  replies `LinkedTerminalReady { request_id, terminal_pane_id }`.
- **`RunInLinkedTerminal`** — writes a command to the linked terminal's PTY
  (optionally echoed). The terminal runs it as if typed.
- **`InsertPathToken`** — injects a path at the terminal's cursor, optionally
  preceded by Ctrl-W to clear the current word.
- **`RequestCommandPreview`** — returns the cwd a command *would* run in, without
  executing it. The host replies `CommandPreview { request_id, command, would_run_in_cwd }`.
- **`OpenArtifact`** — opens a path with the OS handler (`open`) or reveals it in
  Finder (`open -R`). The path must resolve inside the app's `workspace_root`.

## Permission error contract

The denial surface is **principled, not arbitrary**, and follows the operation
kind:

- **Request-response operations carry a `request_id`.** On denial the host emits
  the operation's normal response event with a **sentinel value**, so the SDK's
  blocking helper unblocks and raises a capability-denied error instead of
  hanging forever:
  - `RequestLinkedTerminal` → `LinkedTerminalReady { terminal_pane_id: 0 }`
    (pane id `0` = "no terminal opened").
  - `RequestCommandPreview` → `CommandPreview { would_run_in_cwd: "" }`
    (empty cwd = "nothing previewed").
- **Fire-and-forget operations have no `request_id`** and no response shape. On
  denial they are **dropped silently** (a `warn` line lands in the host log, but
  the app receives nothing — it never awaited a reply):
  `RunInLinkedTerminal`, `InsertPathToken`, `OpenArtifact`.

A denied operation never enqueues a host effect. Regression tests:
`src/process_app/tests/canvas_bindings_tests.rs` covers all five denial paths and
all five granted paths.

## Lifecycle (current behavior)

- **Obtain:** an app calls `RequestLinkedTerminal`; the host opens the terminal
  pane and sets `linked_pane_id` on the app pane.
- **Reuse:** `RunInLinkedTerminal` / `InsertPathToken` target `terminal_pane_id`.
- **Close app pane:** the linked terminal is **left open** (orphaned). Closing
  the app does not close its terminal.
- **Close the terminal:** the app's `linked_pane_id` is **not cleared** (it goes
  stale). A subsequent `RunInLinkedTerminal` to the dead pane id is a graceful
  no-op (the host finds no such pane and drops it).

This "loose coupling" is the current contract: the link is a routing hint, not a
lifetime binding. Whether app/terminal should close together (paired close) and
whether a closed terminal should clear the link are open design questions tracked
as gaps below — they are intentionally **not** changed here.

## Pane grouping

`pane_group` is set on the app pane at spawn (`registry.group_for()`) and is used
for **`PathChanged` (cwd) routing** — directing a terminal's directory changes to
the grouped app. It is **not** wired into visual chrome: panes in the same group
are not rendered side-by-side or tab-grouped, and there is no paired-close. Visual
grouping is future work (see gaps).

## Path safety (`OpenArtifact`)

`OpenArtifact` validates that the target resolves inside the app's
`workspace_root` before shelling out. Validation is **lexical**: relative paths
are joined to the root and `..` segments are collapsed without touching the
filesystem (`normalize_path`). Builtin apps have no `workspace_root` and are
trusted. **Symlinks are not resolved** — a symlink inside the workspace that
points outside passes the lexical check. See gaps.

## Logging & inspection

- Every operation logs at the routing boundary (`ProcessApp[...]: <Op> ...`) and
  denials log a `warn` with the reason.
- `plexi pane list` / `plexi pane info <id>` show the app pane and its linked
  terminal; host effects (terminal created, command written) log under
  `app::<app_id>`.

## Known gaps (tracked, not fixed here)

These are deliberate design questions, filed as issues rather than changed inline:

- **Lifecycle / paired close & stale link** — should closing the terminal clear
  the app's `linked_pane_id`, and should app/terminal close together? Plus the
  visual grouping that `pane_group` does not yet drive.
- **`OpenArtifact` symlink boundary** — lexical validation does not resolve
  symlinks, so a workspace-internal symlink can escape the root.

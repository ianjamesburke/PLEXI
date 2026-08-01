# Context-Scoped State Persistence — Audit Findings

**Stint:** 0678 (this audit), 0680, 0681, 0682, 0683 (the fixes it justifies — the last one to close is this doc's delete trigger)
**Status:** active
**Date of evidence:** 2026-07-31, alpha at the branch point of this task

An audit of why app state — todo's in particular — does not survive a restart.
Everything below is reproduced, not reasoned: each answer names the test, the
log trace, or the bytes on disk that produced it. Where alpha's behaviour and
stint 0652's behaviour differ, both are stated separately; 0652 is **not
merged** (see [Interaction with 0652](#interaction-with-0652)).

## The answer in one paragraph

State loss is not one bug. Three independent defects sit on the same path, and
each of them alone is sufficient to lose a todo item. In order of how early
they bite: **(1)** a Python app pane is not restored at all on the next boot —
it is saved under its *runtime kind* (`python-wasm`), never its app id, so
restore cannot rebuild it and substitutes a terminal; **(2)** a re-launched app
resolves its state file from the *launch-time* `workspace_root`, so an item
written under one root is invisible from another and nothing merges or warns;
**(3)** when two instances of one app are live under the same root — the normal
case, because contexts freely share a root — they address one file with no
coordination, and whichever instance persists last writes its own launch-time
map over the other's. None of the three is a write that never happens: the
write always happens. Two of them are writes to a path nobody reads next, and
one is a read that never occurs because the app is never relaunched.

## Method

- **Rust tests** (all deterministic, all currently green on this branch;
  they assert what alpha *does*, so a fix is expected to change them):
  `audit_0678_two_contexts_accept_the_same_root` and
  `audit_0678_set_context_root_does_not_move_a_live_app_pane_root` in
  `src/app/tests/context_tests.rs`;
  `audit_0678_second_instance_clobbers_the_first_instances_items` and
  `audit_0678_state_written_under_one_root_is_invisible_under_another` in
  `src/host/wasm_python.rs`;
  `audit_0678_wasm_app_panes_are_not_restorable_from_a_saved_workspace` in
  `src/pane_ops/create.rs`.
- **Live host logs**: `~/.plexi-alpha/plexi.log` and `~/.plexi-beta/plexi.log`
  from Ian's running sessions on the evidence date
  (`launch_app_by_id_with_layout`, `workspace_restore`, `app::todo: persisted
  WASM Python state`).
- **Bytes on disk**: the saved workspace files under each channel profile's
  `workspaces/` dir, and every `app_states/` directory beneath the home
  directory.

Nothing here was concluded from reading code alone. Two structural facts
(`AppRuntime::type_id` values, the absence of a uniqueness check on the create
path) are code facts, and each is pinned by one of the tests above.

## The resolution chain

**Write** — `PersistState` from the guest → `save_app_state` message →
`LivePythonPane::save_state` → `python_state_path(&self.config)` →
`write_python_state_atomic`. The path is
`<config.workspace_root>/.plexi/app_states/<app_id>.json`. `config` is the
`PythonLaunchConfig` captured at launch; nothing mutates `workspace_root`
afterwards. There is exactly one write path.

**Read** — `LivePythonPane::launch` → `load_python_state` →
`python_state_path` (same value as the write) → on miss only, a second
candidate: `<parent of config_dir()>/.plexi/app_states/<app_id>.json`, the
channel-neutral global fallback. Both candidates run
`reclaim_python_state_orphans` first, which adopts or archives
`<root>/.plexi-<channel>/app_states/<app_id>.json` copies left by builds that
wrote channel-suffixed paths. **Read has two candidates; write has one.** An
app that loaded from the global fallback persists to the workspace path, so its
next launch under a different root reads the stale global copy again.

**Where `workspace_root` comes from**, in resolution order at launch:
`AppRegistry::InstalledApp::workspace_root` when the app was found inside a
workspace (`resolve_workspace_root_with_channel` walks up from the cwd to the
nearest channel-dir anchor), else the launch `cwd`, which
`resolve_new_pane_cwd` resolves as *context root → focused pane cwd → home*.
For a globally installed app such as todo, `InstalledApp::workspace_root` is
`None`, so the context root wins in practice — and a context with no root falls
through to whatever directory the focused pane happened to be in.

**The divergent copies.** `PythonLaunchConfig::workspace_root` (launch-time,
owns the state path *and* the fs jail), `AppPane::workspace_root` (launch-time,
what workspace-save records as the pane's `cwd`), and `Context::root`
(mutable, what `set_context_root` changes). Only the third can change while an
app is running, and it steers nothing that a running app reads or writes.

## The five questions

### 1. Can two contexts hold the same root? Is there a uniqueness check?

**Yes, and no.** `set_context_root` logs, calls `auto_init_workspace`, assigns
`root`, and returns — it never inspects the other contexts. Neither does the
create path: `create_context` over IPC routes to `new_context_at_path` (root =
the given path) or `new_context`, and `new_context_empty` anchors **every**
new context at the home directory unconditionally, so a second rootless context
duplicates the first by construction. A sub-context inherits its parent's path
when no root is passed.

*Evidence.* `audit_0678_two_contexts_accept_the_same_root` sets one directory
as the root of two contexts through the production entry point; both accept.
On disk, `~/.plexi-alpha/workspaces/default.json` holds contexts `Default` and
`Context 2` both rooted at `/Users/ianburke` — a duplicate root in a live saved
workspace, not a constructed one.

### 2. Two panes of one app — how many state files, and who wins?

**One file, and the last instance to persist wins with whatever it loaded at
its own launch.** The state path is a pure function of `app_id` and
`workspace_root`; no pane id, instance counter, or context id enters it. Two
instances under one root therefore share a file, and each keeps its own
in-memory `persisted_state` that it serializes whole on every `PersistState` —
there is no merge, no re-read before write, and no last-write-wins detection.

*Evidence.* `audit_0678_second_instance_clobbers_the_first_instances_items`
replays the exact host sequence through the production functions: both
instances load empty, the first persists an item, the second persists its
launch-time (empty) map, and the item is gone. Across contexts the answer is
the same whenever the two contexts share a root (question 1), and different
files when they do not (question 4). Live: `~/.plexi-alpha/workspaces/default.json`
records two `python-wasm` panes named `todo`, in *different* contexts
(`Default` and `Context 2`), both with `cwd` `/Users/ianburke` — two live
instances addressing one file, exactly the shape the test reproduces. The alpha
log for the same session shows the hot-reload watcher firing separately for each
of those pane ids.

### 3. Does `set-root` after launch move a running app?

**No. A running app keeps its launch-time root.** `set_context_root` mutates
`Context::root` and, only for the active context, re-runs
`apply_context_transition_effects` (registry rescan, watcher restart, config
reload). No pane's `workspace_root` and no `PythonLaunchConfig` is touched, so
a live app keeps reading and writing where it launched while *new* launches in
that context go somewhere else. The CLI half-says this already — `plexi context
set-root` prints a tip that `PLEXI_CONTEXT_ROOT` is only picked up by newly
opened panes — but nothing says it about app state.

*Evidence.* `audit_0678_set_context_root_does_not_move_a_live_app_pane_root`
records a live app pane's `workspace_root`, changes its context's root through
the production entry point, and asserts the pane's root is unchanged.

### 4. Where do todo's bytes go right now, and which path is read next?

**Written to `<launch root>/.plexi/app_states/todo.json`.** On the evidence
date, with the context root at the home directory, that is
`~/.plexi/app_states/todo.json` — which is *also* the global fallback path, so
under a home-rooted context the two read candidates coincide and the fallback
is invisible. Under any other root they diverge.

*Evidence — bytes on disk.* Every `app_states/todo.json` beneath the home
directory at audit time, with mtimes: `~/.plexi/app_states/todo.json`
(2026-07-31 17:48, one item), `~/Documents/GitHub/nooise/.plexi/app_states/todo.json`
(2026-07-30 21:47, empty item list),
`~/Documents/GitHub/PLEXI/.plexi-beta/app_states/todo.json` (2026-06-28),
`~/Documents/GitHub/stint/.plexi-pr-2323/app_states/todo.json`,
`~/Documents/GitHub/narrator-ai-v1/.plexi-pr-2323/app_states/todo.json`, plus
`.superseded` archives under `~/.plexi-beta/` and `~/.plexi-alpha/` left by the
orphan reclaim. The launch traces that produced the first two are in the logs:
`launch_app_by_id_with_layout: id=todo cwd="/Users/ianburke/Documents/GitHub/nooise"`
(2026-07-30) and `... cwd="/Users/ianburke"` (2026-07-31). Same app id, same
user, one afternoon apart, two files — and neither launch could see the other's
items.

**Which one is read on the next launch** depends on the context root at that
moment, which is the whole defect: it is not a property of the app.

### 5. Is the loss a write that never happens, a write nobody reads, or a read that resolves elsewhere?

**Never the first. Both of the others, plus a fourth shape the question did not
anticipate: a read that never happens at all.**

- *Read that never happens* — after a restart the app is not relaunched.
  Workspace save records an app pane's `app_id` as `AppRuntime::type_id()`,
  which is `python-wasm` for every CPython app and `wasm` for every native one;
  `serialize_state` returns `None` for both, so `app_state` is saved as null.
  Restore looks the saved id up in `builtin_factory`, finds nothing, and the
  caller substitutes a `TerminalPane` in the pane's place. The state file is
  never opened. *Evidence:*
  `audit_0678_wasm_app_panes_are_not_restorable_from_a_saved_workspace`; and
  across both channel logs, every `workspace_restore` line names a builtin
  (`text-editor`) or the assistant — there is not one restored `python-wasm` or
  `wasm` pane in either log's entire history.
- *Write to a path nobody reads* — a relaunch under a different root writes and
  reads a different file. *Evidence:*
  `audit_0678_state_written_under_one_root_is_invisible_under_another`, and the
  nooise/home file pair above.
- *Read that resolves elsewhere* — the global fallback is read-only: an app that
  loads from `<parent of config_dir()>/.plexi/app_states/` persists to its
  workspace path, and can load the same stale global bytes again on a later
  launch under another root.

## The confirmed root cause

**App state is addressed by the launch-time filesystem root of the pane, and
nothing in the system guarantees that address is stable, unique, or revisited.**
Every symptom follows: contexts duplicate roots freely, so one address serves
several instances; the address is captured at launch, so moving the context does
not move the app; and the pane that owned the address is not rebuilt at boot, so
the address is not even re-read. The item is written every time. It is the
addressing that fails, in three separable places.

## Interaction with 0652

Stints 0651 and 0652 are on **PR #2536, which is open, conflicting, and not
merged to alpha.** Everything in the sections above is shipped alpha behaviour.
This section describes where 0652 *lands* the resolution rules; it does not
re-litigate them and the audit does not depend on them.

0652 introduces `src/host/state_scope.rs` as the single owner of path
construction, with two rules: `global` →
`~/.plexi/app_states/<app_id>.<ext>`, `context` →
`<context.root>/.plexi/app_states/<app_id>.<ext>`. An app declares its scopes
in the manifest (`[state] scopes`); an omitted `[state]` means `["global"]`.
Resolution moves to **call time** against the pane's live context root, pushed
into each app pane every frame, so `set-root` redirects context-scoped state
immediately. 0651 makes `Context::root` non-optional, which removes the
rootless-context fallback that lets a launch cwd leak into the address.

Mapped onto the five questions:

- **Q3 is fixed by 0652** for context-scoped apps: call-time resolution is
  exactly the missing half. The launch-captured `workspace_root` survives only
  as the fs jail.
- **Q4/Q5 second shape are fixed for todo specifically, by accident of the
  default**: todo's manifest declares no `[state]`, so on that branch it
  resolves `global` — one file for the whole machine, no root in the address.
  Any app that declares `context` keeps the per-root split, which is the
  intended behaviour, not a defect.
- **Q1 is untouched.** 0651 makes every context have a root; nothing makes it
  unique. Two contexts rooted at one directory still share one context-scoped
  file.
- **Q2 is untouched.** Scope is a property of the app, not of an instance —
  by design — so two live instances still address one file and still overwrite
  each other whole. 0652's own doc comment states the design intent; the
  multi-instance write conflict is out of its scope.
- **Q5 first shape is untouched.** 0652 changes addressing, not workspace
  restore. A python-wasm pane still cannot be rebuilt at boot.

The fixes below are therefore written to sit *on top of* 0652 where they
overlap, and independently of it where they do not.

## Follow-on stints

Filed from these findings, one per distinct defect. None of them is in scope
for 0678.

| Stint | Defect | Question |
|---|---|---|
| 0680 | WASM/Python app panes are saved under their runtime kind and are not restored — the pane returns as a terminal | Q5 |
| 0681 | Two live instances of one app overwrite each other's state whole; no merge, no reload-before-write, no conflict signal | Q2 |
| 0682 | The state read path has a second candidate the write path does not, so a load can resolve to bytes the app will never write back | Q5 |
| 0683 | State reads and writes are unobservable — the persist trace names no path, so where an app's bytes went cannot be answered from the log | method |

The duplicate-root rule that question 1 exposes is a design question, not a bug
fix; it is owned by stint 0679's design brief and awaits Ian's ruling.

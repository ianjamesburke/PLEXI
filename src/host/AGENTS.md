# src/host — Agent Contract

**Read before editing anything under `src/host/`:** this file, plus the root
`AGENTS.md`.

## The three scope models

Plexi has three scope models. They are not variants of one thing, and picking the
wrong one is how cross-context leaks and per-channel data forks get built. Decide
which class your resource is in *before* writing path or visibility code.

| Model | Owns | Keyed on | Channel | Tiers relate by |
|---|---|---|---|---|
| `scope.rs` — `ScopeOrigin` / `Scope` / `evaluate_reach` | in-memory and wire resources: connector tools, event streams, directed and typed pipes, notifications | `context_id` (runtime reachability) | n/a | nothing; a resource is owned in exactly one context |
| `state_scope.rs` — `StateScope` / `UserDataKind` | files a user owns: app state, notes | canonical context root (a file address) | **neutral** `.plexi` | accumulation; both tiers are real and both are read |
| `../app/registry.rs` — `RegistrySource` | layered config discovery: apps, agents | workspace root walked up from cwd | **scoped** `workspace_channel_dir()` | shadowing; a local entry replaces a global one of the same id |

Three rules follow from the table, and each one has been violated at least once:

- **Config may fork per channel; user data must not.** An app definition is
  config, so it lives under `workspace_channel_dir()`. A note or an app's saved
  state is the user's, so it lives under the channel-neutral `.plexi` — otherwise
  running a beta or PR build silently splits their data in two.
- **Config entries shadow; documents accumulate.** Shadowing needs a stable id to
  shadow *by*. Notes have no such id, so a note present in two tiers is not a
  precedence question, and both tiers are listed. Where an ambiguity genuinely
  cannot be resolved (two notes with the same wiki name), withhold and report
  every candidate — never silently pick a winner. `ToolRegistry::snapshot_for_caller`
  is the precedent.
- **`evaluate_reach` is not a file-visibility predicate.** It answers "may a
  caller in this context see a resource owned in that one" for live, in-process
  resources. A file is addressed, not reached; `state_scope.rs` is deliberately
  outside `evaluate_reach` and that is not a gap.

### Adding a kind to `state_scope.rs`

Add a `UserDataKind` variant and route every path through `user_data_dir`. Do not
build a parallel resolver, and do not rename an existing kind's directory — those
names are user-data addresses. `assert_within_scope` is kind-aware and must be
called before any write: it rejects a symlinked `.plexi/` or `<kind>/` that would
redirect writes outside the scope.

### Do not resurrect the deleted layered-source resolver

`ResolvedSource<T>`, `resolve_layers`, and `Scope::{Global, Context, Window}` were
built in stint 0724 and deleted in the same PR under
`cargo clippy --bin plexi -- -D warnings` because nothing constructed them. Stint
0744 is archived "resolved by deletion," and its ruling stands: a future layered
-source consumer designs against its own requirements rather than restoring the
unused version. Two consequences for anyone adding scope code here:

- Run `cargo clippy --bin plexi -- -D warnings`, not just `cargo build`. Every
  phase of 0724 passed local builds and was caught by that gate at the end.
- Speculative generality in this module gets deleted, so add the variant when the
  consumer exists — not before.

## Why an env var can never carry current scope

`PLEXI_CONTEXT_ROOT` is stamped into a pane's environment when the pane spawns, and
a parent process cannot mutate a running child's environment. After
`plexi context set-root`, every long-lived pane shell holds a permanently stale
copy. Anything that must be *current* therefore has exactly two options:

- ask the host, which owns the live router; or
- derive it from the working directory, which is always current — the nearest
  ancestor holding a `.plexi` directory (`notes::anchored_root_for`).

The second is why notes and app state are host-independent from the CLI without
being wrong: their addresses are predictable from outside the process. This is the
answer to stint 0745's open question — that split is correct, not a missing
migration. `ScopeOrigin`'s module doc bans env vars as authority inputs for the
same reason.

## Traps

- **`shared_dir()`, never `home_dir().join(".plexi")`.** `crate::config::shared_dir()`
  carries a thread-local test override; re-deriving the path by hand produces a
  tier no test can isolate, so unit tests write into the developer's real
  `~/.plexi`.
- **A test that touches a user-data tier needs a `shared_dir` override, not just a
  profile override.** `set_test_profile_dir` isolates `config_dir()`, which is
  channel-scoped — it does nothing for the channel-neutral shared dir the tiers
  live in. `HostHarness` and `PlexiUiHarness` install
  `crate::config::set_test_shared_dir` for their lifetime; a plain unit test must
  install one itself. This is not theoretical: stint 0746's own test runs wrote
  files into a real `~/.plexi/notes/` before the guards existed. Any new kind added
  to `state_scope.rs` inherits the same hazard.
- **The symlink guard depends on canonicalizing only the base.** `scope_layout`
  returns a trusted base plus literal tail components on purpose: resolving the
  whole expected path would accept a symlinked `.plexi/` instead of rejecting it.
- **A context rooted at the home directory has the global tier as its context
  tier.** `new_context_empty` produces exactly that, so any code listing both
  tiers must dedupe canonically or it shows the same directory twice.

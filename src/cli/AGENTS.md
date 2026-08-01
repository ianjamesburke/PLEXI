# src/cli — Agent Contract

**Read before editing anything under `src/cli/`:** this file, plus the root `AGENTS.md`.

## Reference

- [`src/render/CLI_APP_CONTRACT.md`](../render/CLI_APP_CONTRACT.md) — CLI-backed app runtime contract (launch, lifecycle, caching, permissions).
- [`registry/CLI_DESCRIPTOR_GUIDE.md`](../../registry/CLI_DESCRIPTOR_GUIDE.md) — CLI descriptor authoring guide (field reference, `ui_hint`, verification).

## Channel-Agnostic CLI Rule

Every CLI command and feature must work identically on alpha, beta, main, and PR builds. The release channel is an implementation detail. See `scripts/RELEASE_CHANNELS.md` for the channel table and shim behavior.

**Path rules:** Never hardcode a profile directory path — always use `config_dir()`. Never hardcode `.plexi/` as a workspace dir — always use `workspace_channel_dir()` or `workspace_config_path()`.

**Socket rule:** A channel-suffixed binary always sends commands to its own profile's `notify.sock`, even when it inherits a different `PLEXI_SOCKET`. The bare `plexi` binary honors `PLEXI_SOCKET`. Route new command dispatch through `resolve_command_socket()`.

**Transport completion is newline-framed.** The CLI reports socket success only after the complete newline-delimited JSON frame is accepted. The host discards EOF-terminated partial frames, even when the partial bytes form valid JSON. An explicitly reported incomplete-frame transport failure was not dispatched and is safe to retry; a later host response timeout is not proof of non-delivery.

**Completion testing on PR builds:** `just pr-install` skips completion install. To test, manually run `plexi-pr-<N> completions zsh > <path>` and restore after.

## CLI Design Rules

- **Namespace design:** verify a new command belongs in the right namespace. Place it where the noun already lives, not at top level.
- **Pane naming:** always name panes after spawning them. Every `plexi pane new`, `plexi app open`, split, or new window should be followed by `plexi pane name <id> "descriptive name"`.
- **Tips:** use `print_tip()` from `src/cli/mod.rs`. Never raw `eprintln!`. Respects `config.cli.tips` and `NO_COLOR`.
- **An absent flag must reach the host as absent.** A clap `default_value` on an optional flag means the CLI can never send "unset", so a host-side default is dead code and changing it silently does nothing. `plexi notify --scope` carried `default_value = "global"`, which is why the host's fallback was unreachable. Leave `Option<String>` flags without a `default_value` and let the host resolve the default (`NotifyScope::default()`); map every *named* value explicitly, never by falling through to the host's fallback.
- **Repeatable flags fan out or they error.** A count flag paired with a repeatable value flag (`context sub --agents N --command CMD`) accepts the value once (applies to all) or exactly N times (one each). Any other count is a hard error — never truncate, never cycle. Expand to one entry per unit in the CLI so the host receives an unambiguous list; see `expand_pane_commands`.
- **Resolve the caller's context by `PLEXI_CONTEXT_ID`, not `PLEXI_CONTEXT_NAME`.** Context names are not unique; the id is. Send the id; the host consults the name only when no id was sent (`resolve_parent_context`). An id naming no live context is an error, never a reason to fall back to the name — a stale id plus a name some other context happens to share would silently target a stranger. The same identity is also provenance: a command whose effect is attributed to its sender (`plexi notify`) stamps `PLEXI_CONTEXT_ID`/`PLEXI_PANE_ID` onto the request itself, because the host must never derive the sender from dispatch-time active state; an unresolvable identity widens the effect (notify escalates scope to global) rather than fabricating a home.
- **Agent-facing commands answer in one round trip.** A command that creates addressable objects returns their ids in its JSON response (`context sub` → `{"context_id","windows","panes":[…]}`), so the caller never needs a follow-up `pane list` to act on what it just made.
- **Blocking orchestration verbs answer from the host, never a CLI sleep loop.** `pane send --submit`, `pane new --agent`, and `pane slot wait` park their reply host-side (`src/app/pane_wait.rs`), serviced from `App::logic` with a host deadline strictly below the CLI's poll window so the caller always sees the host's typed reason, not a generic client timeout. Exit codes are the contract — `0` confirmed outcome, `2` timeout (distinct, branchable), `1` usage/plumbing — and stdout carries only the payload (pane id, matched value). Waits are level-triggered: a condition already true at call time answers immediately. A new wait-shaped verb follows this pattern; never add a CLI-side poll loop wearing a blocking coat.
- **`pane command --enter` shares `pane send --submit`'s settle → Enter → confirm path.** Writing `<cmd>\n` as one raw blob races an interactive shell's completion/autosuggest machinery for the Enter (stint 0654; observed as an ambiguous-prefix alias opening a completion menu instead of running). `--enter` therefore maps 1:1 onto `SendToPane { submit: true }` — the exact request `pane send --submit` issues — through `cli::pane_send_cli`, not a second settle implementation. It is scoped to terminal panes only (an app pane refuses `--enter` the same way it refuses `--submit`), and its exit code is the same contract: `0` confirmed submitted, non-zero typed-but-unconfirmed with the observed input line on stderr. Without `--enter` it stays the plain type-only verb (`submit: false`).
- **Background host launch is explicit boot state.** `plexi host start --background` launches without activating Plexi; on macOS it uses Accessory policy and has no normal Dock or menu-bar presence. The flag is forwarded through a launch-only env var that the host consumes at process entry, before it can reach terminal/app descendants; the CLI also strips it before explicit re-add, so background mode never cascades into a nested `host start`. It is not configuration. App Nap exemption remains unconditional. Pane send/key/click and `pane focus` operate inside the host through egui/PTY paths and do not request OS activation; `pane focus` changes focus within Plexi but does not front an Accessory app.
- **Pane capture cursors account for screen redraws, not only shell linefeeds.** Full-screen alternate-screen TUIs repaint rows in place while leaving the terminal cursor on the same line. `pane capture --from-cursor` must advance for those redraws and return the changed rows; `--plain` prints only captured lines on stdout and puts the next `cursor=<N>` on stderr. The default JSON envelope stays unchanged.
- **Pane status is the host-owned corroboration path.** `pane status <id>` combines the shared agent detector, the TUI status bar, and the trailing buffer line into `working | idle | blocked | unknown` plus `high | low` confidence and the raw evidence. Working agent state and a trailing tool call are authoritative; idle requires all signals to agree; a truncated status bar can never prove the busy marker is absent.
- **Pane heartbeats are host-owned.** `pane heartbeat <id> --every <duration> --text <text>` schedules type-and-submit work in `App::logic`; it never sleeps or loops in the CLI. The default only fires on idle agent panes, `--off` removes it, and status/list report its additive state.
- **Boot state comes from the agent detector, never a PTY regex.** `pane new --agent` blocks on the pane's `PaneAgentState` reaching idle through `Pane::agent()`, which merges the detector's two signal sources: hook reports (`SetAgentState`, authoritative once any hook fires) and the host-observed synthesis in `tick_terminal_activity` (known agent binary in the PTY foreground + output settle — required because Codex defers `session_start` to the first prompt submission, so a freshly booted Codex never self-reports). New agent support goes into that one detector (`known_agent_process`, the hook installer), never into a parallel prompt-signature scraper.
- **Reported agent identity is human-facing.** The `agent report --agent` name and `--detail` active tool appear in Cmd+P across all contexts. Same-name agents are distinguished by their one-based position within the context, so repeated squad commands remain navigable.

## Documentation Rule for CLI Changes

Any change to a CLI verb, flag, or agent-facing behavior updates this file **and** `skills/plexi-cli/SKILL.md` in the same PR. Edit the skill through the real `skills/` path — `.agents/skills/plexi-cli` is a symlink and some editors do not write through it. Shell completions are generated from the clap tree (`completions_cli`), so a new flag needs no hand-written block; verify with `plexi completions zsh | grep <flag>`.

## Traps

- **Path-based app commands must not resolve a workspace.** `app validate <path>`, `app install <path>`, `app run <path>` operate on an explicit filesystem path. Never call `resolve_workspace_root` for that argument. Use `std::fs::canonicalize` directly. `resolve_workspace_root` is only legitimate in `AppRegistry::load` and `app init`.
- **Profile reconciliation is narrowly scoped.** `app prune --dry-run` reports only positively identified retired first-party pre-v3 installs; never infer deletability from absence from the current core pack, because user and marketplace apps also live in the global profile.
- **Building a `-c` command string:** use `cmd_from_args` (in `src/app/mod.rs`), not `shell_join` directly. A single-arg array is already a shell expression; `shell_join(["echo hello"])` yields `'echo hello'`.
- **Shell suffix construction:** when appending a stay-alive or exec suffix to a user command string, use the absolute shell path from `settings.shell` (already resolved), not `$SHELL`. `trim_end_matches([';', ' '])` the user command before appending to prevent `;;` syntax errors.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.

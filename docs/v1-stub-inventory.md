# V1 stub inventory

Snapshot 2026-10-06 (alpha `9bec432b`). This file is an inventory, not a PRM and not a progress tracker. Follow-up agents should treat the open PRs as the live state.

Scope is the agents V1 contract, including Ian's 2026-10-06 decisions: agents V1 ships alpha → beta (stable stays the 0.3 host); tamper-evident, not a container; text change sets only; app tools over host MCP through the gate; no lead-to-lead messaging; non-sensitive app capabilities auto-grant and sensitive ones need a click; a second lead is created from the CLI and from a New lead button. Out of V1: change sets over MCP, lead-to-lead messaging, XML and video editors, container isolation, the house agent, Touch ID, marketplace upload.

Alpha has no `todo!()` or `unimplemented!()` outside comments. `src/main.rs` denies both. The blockers below are commands that record or print and do not do the thing the contract names, in-memory state the contract says must survive restart, e2e scripts that approve through the CLI, and failing CI.

Line numbers are the tree named in the row. `covered` means an open PR already contains the fix or the port; it does not mean the item is DONE. DONE still means the contract's installed test, with human clicks from `scripts/e2e/human.sh`, on an alpha-based build.

## Findings

| id | where | what's missing | blocks | covered by | size |
|---|---|---|---|---|---|
| F01 | alpha `src/app/app_call.rs:64` `observe_permissions` match `"resolve"` | Any socket caller can `approve_pending`. No caller check. | V1-03, V1-02 | #2720 refuses this verb. #2704 / #2713 / #2708 do not. | L |
| F02 | alpha `src/app/lifecycle.rs:1560` `KeyPane`; click handler follows in the same function | `pane key` and `pane click` are not tagged synthetic, so they can satisfy an approval. | V1-03, V1-12 step 7 | #2720 (`refuse_synthetic_approval` in `app_call.rs`) | M |
| F03 | alpha `skills/plexi-cli/SKILL.md:68` (also the four skill symlinks) | Teaches `assistant permission resolve`. `plexi_version` is `0.3.5`. No binary install path. | V1-14, V1-03 step 10 | #2722 embeds the skill and drops the resolve instruction | S |
| F04 | alpha `src/broker/gate.rs:72` and `:306` `persist_pending` | Pending approvals live in a process `Mutex`. Restart drops them. | V1-05 step 3, V1-09 restart | #2713 writes `<profile>/host` journal. #2704 is still in-memory. | M |
| F05 | alpha `src/pane_ops/create.rs:2138` | A second Assistant in the same context is focused away. One conversation per context. | V1-10 | #2715 `open_assistant_for_head` plus New lead in `src/assistant/render.rs` | M |
| F06 | alpha `src/app/lifecycle.rs:219` `SubmitAssistantTurn` | `assistant send` walks panes and submits to the first Assistant. No `--head`. | V1-10 | #2715 | S |
| F07 | alpha `src/plexi_ai/broker.rs:815` and `:939` | Missing provider usage becomes `unwrap_or(0)` and is stored as `Some(0)`. No `plexi ledger` verb (`src/cli/args.rs` has no `Ledger`). | V1-07 | #2710 keeps unknown totals and adds `Ledger` at `src/cli/args.rs:171`. Real OpenRouter turn not in the mock e2e. | S |
| F08 | alpha `src/broker/mod.rs:641` `GrantStore` | `grants.toml` is a plain file an agent can append. Legacy `permissions.toml` is still loaded beside it. Non-sensitive capabilities auto-grant in `src/app/permissions.rs:634` and are not rows of this store. No HMAC. | V1-04, V1-15 | #2718 (`src/broker/seal.rs`, `scripts/permissions-seal-e2e.sh`, `scripts/app-share-e2e.sh`) stacked on spike #2700, not on alpha | L |
| F09 | #2706 `src/agent/heads.rs` `issue_run` (spawn body) and `handle_request` `"spawn_run"` | `agent run spawn` appends a run record and a caller-supplied token count. It does not call a model. Run tokens sit in a process `Mutex` (`tokens()` near the top of the same file), so a restart rejects them — that part matches V1-09 step 4. | V1-09 is records; V1-10 is the real turn | The model loop is a different path: #2715 `src/agent/leads.rs` `submit_turn` / `run_model_turn`. `spawn_run` stays a journal. | S |
| F10 | #2706 and #2715 `src/agent/heads.rs` `handle_request` `"create_head"` | CLI create passes `parent_grants: None` and `denial_actor: "operator"`. An agent pane can mint a head whose grants are wider than its own. Subset checks run only for `delegate`. | V1-09 step 5 | Not covered. #2715 still passes `parent_grants: None` on the CLI create arm. | M |
| F11 | #2707 `src/host/command_view.rs` `CommandBoard::send` / `cancel`; board is `OnceLock<Mutex<…>>` at `shared()` | Send, enqueue, pause, and cancel rewrite an in-memory board. Nothing starts or stops a model turn. Restart drops the board. | V1-11 | #2717 `lifecycle` `CommandView` `"send"` calls `leads::submit_turn`; `"cancel"` calls `queue::request_cancel_run`. Do not merge #2707 as the control plane. | M |
| F12 | #2707 `src/host/command_view.rs` `handle` arms `"resolve"` and `"allow"` | Those arms call `settle_needs_you` and can grant. | V1-03, V1-11 step 5 | #2717 `command_view_refused` in `src/cli/agents_api.rs` does not grant. | S |
| F13 | #2704 `src/app/app_call.rs:103` `observe_needs_you` `"resolve"`; same function on #2713; #2708 adds `from_phone` and still calls `resolve_needs_you` for everyone else | `needs-you resolve` from a pane grants. #2720's `NeedsYou` command only forwards to the refused assistant-permission verb and does not contain this function. | V1-03 step 3, V1-05 step 5 | Not covered on the needs-you or relay trees. | M |
| F14 | #2704 `scripts/needs-you-e2e.sh:160`; #2713 same line; #2706 `scripts/e2e_agents_api_installed.sh:215`; #2695 `scripts/assistant-editor-change-set-e2e.sh:65`; #2705 `scripts/folder-secrets-e2e.sh:297` | Positive path is `needs-you resolve`, `assistant permission resolve`, or `secret grant` treated as success. | V1-02, V1-05, V1-06, V1-09, V1-12 | #2709 adds `scripts/e2e/human.sh` (`xdotool`, not `pane click`). #2714's `scripts/permission-gate-e2e.sh` already calls `HUMAN_APPROVE`. The feature scripts above do not. | M |
| F15 | #2715 `scripts/multi-lead-e2e.sh:5`; #2716 `scripts/headless-queue-e2e.sh:4`; #2717 `scripts/command-view-steer-e2e.sh:6` | Each script prints `VERIFIED-VIA-BYPASS` and never calls `HUMAN_APPROVE`. | V1-10, V1-11 (cannot be DONE) | #2709 driver exists; these scripts do not use it. | S |
| F16 | #2705 `scripts/folder-secrets-e2e.sh:230` and #2719 `:256` | `PLEXI_E2E_SKIP_PANES=1` prints `SKIP` and skips the pane half. | V1-06 steps 2–3 | #2719 closes the CLI bypass (`src/cli/workspace.rs` `folder_secret_grant` prints `permission_denied` and does not call `grant_folder_secret`). The skip flag remains. | S |
| F17 | #2719 `src/cli/workspace.rs` `folder_secret_grant` | Comment says the host calls `grant_folder_secret` when a person approves. The only production reference assigns the function pointer and does not call it. A read does file a gate pending (`src/workspace/secrets/folder.rs` `read_folder_secret`). No installed script approves that pending with a human click. | V1-06 step 4 | #2719 for the refusal. The human grant is not wired in an e2e. | M |
| F18 | alpha has no `clients/` and no `services/relay/`. Phone stub is #2676 `clients/phone-web/server.py:84` (`Stub echo:`). | No pair code, no sealed relay, no phone Needs you. | V1-08 | #2708. Irreversible phone approve is refused (`src/app/app_call.rs` `waiting on desktop`). Desktop `needs-you resolve` on that branch still grants (F13). | L |
| F19 | alpha has no change-set commit. Spike #2695 `src/app/text_editor_app.rs:514` `commit_prepared`. | No alpha port. Accept is a CLI resolve in the e2e, and `pane key` can hit the editor accept path. | V1-12 | No alpha PR. #2695 / #2697 are spike-stacked. | L |
| F20 | alpha `src/cli/app.rs:377` `MARKETPLACE_PLACEHOLDER`; `src/release.rs:166` | Marketplace publish, DAW, media I/O, and accessibility are gated stubs. `src/app/dispatch.rs:809` rejects layout `background`. `src/app/canvas_bindings.rs:441` falls back to the OS for `cmd:` file handlers. | not V1 | none | — |
| F21 | #2701 `services/house-agent/runner.py` `handle_text` | A non-tool turn returns the canned string `Chess Opponent ready.` The `kind=model` arm hashes a vault key and does not call a provider. | not V1 (house agent is out) | leave #2701 unmerged | — |
| F22 | #2678 connector login is a desktop OAuth spike; #2689 Touch ID; `docs/xml-video-editor.md` | Not on the agents V1 list. | not V1 | none | — |
| F23 | #2679, #2677, #2671, #2618, #2684, #2680 | Open, not on the V1 acceptance list. #2680 is the spike base ("do not merge"). | not V1 | close when the alpha ports replace them | — |

## E2E that cannot mark DONE

| script | ref | why it is not DONE |
|---|---|---|
| `scripts/permission-gate-e2e.sh` | #2714 | Uses `HUMAN_APPROVE`. Still not an installed alpha run, and it cannot be DONE until F01 is refused (V1-03) or the bypass would still exist in the product. |
| `scripts/needs-you-e2e.sh:160` | #2704, #2713 | `needs-you resolve --approve`. |
| `scripts/e2e_agents_api_installed.sh:215` | #2706 | `assistant permission resolve`. |
| `scripts/folder-secrets-e2e.sh:297` | #2705 | `secret grant` is a passing assertion. #2719 inverts the agent-pane case and still skips panes when `PLEXI_E2E_SKIP_PANES=1`. |
| `scripts/assistant-editor-change-set-e2e.sh:65` | #2695 | `assistant permission resolve`. No freeze check (V1-12 step 6). |
| `scripts/multi-lead-e2e.sh`, `scripts/headless-queue-e2e.sh`, `scripts/command-view-steer-e2e.sh` | #2715–#2717 | Marked `VERIFIED-VIA-BYPASS` in the script header. |
| `scripts/v1-acceptance.sh` | #2721 | Harness only. A green run that did not call `HUMAN_APPROVE` is recorded as `VERIFIED-VIA-BYPASS`. |

`scripts/e2e/human.sh` on #2709 is the driver the contract calls W15. It clicks with `xdotool` on the host window. It is not yet sourced by the feature scripts above.

## CI (open PRs, this snapshot)

Pending and queued checks are not failures. Most alpha-port PRs (#2703–#2706, #2708–#2722) still had `test` or `clippy` queued. No check was observed flaky (no rerun oscillation in this rollup).

| PR | head | failed check | log |
|---|---|---|---|
| #2707 | `70139c1b` | `check-capability-docs` | `website/src/content/docs/pgap.md` is stale. The diff adds a `command_view` section. Job [112121583954](https://github.com/ianjamesburke/PLEXI/actions/runs/37418245464/job/112121583954). |
| #2702 | `fd52cd76` | `macos-x64` | Failed step is "Test and lint the host" (job [112119181797](https://github.com/ianjamesburke/PLEXI/actions/runs/37417476001/job/112119181797)). The workflow run was still open, so the log body was not downloadable. `macos-arm64`, `test`, and `clippy` were still queued. |
| #2700 | `e7d9b7f4` | `linux-x64` | Failed step "Assemble and exercise every consumer channel" (job [112124939107](https://github.com/ianjamesburke/PLEXI/actions/runs/37419333827/job/112124939107)). Spike base. Alpha port is #2718. |
| #2701 | `b77b3451` | `linux-x64` | Same step name (job [112118919895](https://github.com/ianjamesburke/PLEXI/actions/runs/37417391865/job/112118919895)). Not V1. |
| #2699 | `fe0038f7` | `linux-x64`, `macos-arm64`, `macos-x64` | Spike command view. Superseded for alpha by #2707 / #2717. |
| #2698 | `955394b0` | those three arch jobs | Spike cloud basics. Alpha port #2703. |
| #2696 | `e718ddeb` | those three | Spike agents API. Alpha port #2706. |
| #2695 | `869ab88b` | `linux-x64`, `macos-arm64` (`macos-x64` still pending) | Spike change sets. No alpha port. |
| #2694 | `8774edcb` | those three | Spike chess isolation. Alpha port #2702. |
| #2691 | `4fec5794` | those three | Spike folder secrets. Alpha port #2705. |
| #2688 | `ad820017` | those three | Spike needs-you. Alpha port #2704. |
| #2686 | `f49e4136` | those three | Spike gate. Merged to alpha as #2690. |
| #2685 | `0f3b5cc8` | `build`, `linux-x64`, `macos-arm64`, `macos-x64`, `windows-x64` | Spike relay. Alpha port #2708. |
| #2683 | `200d10db` | those three arch jobs | Orphan ledger. Alpha port #2710. |
| #2682 | `effb4ee5` | those three | Phone correlation, rides inside #2708. |
| #2680 | `21b6f4aa` | `test`, `check-cli-docs`, `check-schema`, `check-capability-docs`, three arch jobs | Spike base. Do not merge. |
| #2689 | `10a1222b` | `build` plus four arch jobs | Touch ID. Not V1. |
| #2679 | `2003a430` | `test`, `check-capability-docs`, three arch jobs | Not V1. |
| #2678 | `29c80c87` | `test`, `check-cli-docs`, three arch jobs | Connector OAuth stub. Not V1. |
| #2677 | `f19ab9fa` | three arch jobs | Not V1. |
| #2671 | `492778ce` | `clippy` (step "Lint host") | Not V1. |
| #2618 | `7590a186` | `test` (step "Test host") | Not V1. |

#2676, #2681, #2684, #2687, #2692, #2693, #2697, #2714, #2600, #2669, #2673 reported no failing check. #2714 has a single successful check and is not a full host CI matrix.

## Work packages

Packages are cut so two agents do not edit the same files. Shared collision files, which show up in more than one V1 PR already: `src/main.rs`, `src/cli/args.rs`, `src/broker/gate.rs`, `src/app/app_call.rs`, `src/app/lifecycle.rs`, `skills/plexi-cli/SKILL.md`, `src/agent/heads.rs`.

Anything that adds a CLI verb has to be the only agent in `src/main.rs` and `src/cli/args.rs` at that time. The stacks below already touch those files; new work rebases onto the owner instead of editing them in parallel.

| package | findings | owner PR (do not start a second branch) | files this package may edit | other packages must stay out | done on an installed build |
|---|---|---|---|---|---|
| P1 Chess CI | #2702 macos failure | #2702 `plexi-chess-fix-alpha` | chess test helpers only | `src/main.rs`, `src/cli/args.rs`, `src/broker/gate.rs` | `macos-x64` "Test and lint the host" is green. `plexi-pr-2702` still runs the chess gate test. |
| P2 Ledger | F07 | #2710 `plexi-ledger-alpha` | `src/plexi_ai/broker.rs`, `src/plexi_ai/ledger.rs`, ledger CLI already on this branch | gate, lifecycle, skill | `scripts/e2e/ledger/run.sh` against the mock OpenRouter: a turn's row has nonzero tokens or the literal unknown, never `Some(0)`, and `plexi ledger` prints a per-client total. |
| P3 Human driver | F14, F15 | #2709, then one commit on each feature branch **after** that branch's code freeze | `scripts/e2e/human.sh` and the single e2e script named in the row | Rust sources | `grep -nE 'permission resolve\|needs-you resolve\|secret grant' scripts/*e2e*.sh` has no positive-path hit. `HUMAN_APPROVE` commits the chess move. |
| P4 Skill | F03 | #2722, already stacked on #2720 | `skills/plexi-cli/SKILL.md`, `src/cli/skill_install.rs` | gate, heads, editor | `scripts/skill-check.sh`: installed `plexi_version` equals `$B --version`, every named command exists, the resolve grep is empty, a scripted "move e7e5" prints the pending id and does not resolve. |
| P5 No self-approval | F01, F02, F12, F13 | one branch rebased onto #2704 **and** #2720 (today they do not share a base) | `src/app/app_call.rs`, `src/app/lifecycle.rs` KeyPane/ClickPane, `src/broker/gate.rs` refuse path, the `NeedsYou` arm in `src/main.rs` | `src/agent/heads.rs`, `src/plexi_ai/broker.rs`, `text_editor_app.rs`, `skills/plexi-cli/SKILL.md` | `scripts/no-self-approval-e2e.sh` steps 1–10: pane CLI, `--socket`, env-stripped, double-fork, `pane key`, and `pane click` all return `permission_denied` and an audit row; `HUMAN_APPROVE` commits once. |
| P6 Needs-you disk | F04 | #2713 `cursor/persist-needs-you-899b` | `src/broker/needs_you_store.rs` and the load path in `src/broker/gate.rs` | do not also take P5's refuse edit in the same commit | Stop and start the host; the same pending id is still listed. Badge count matches `needs-you list --json` (`host screenshot`). |
| P7 Folder secrets | F16, F17 | #2719 on #2705 | `src/cli/workspace.rs`, `src/workspace/secrets/folder.rs`, `scripts/folder-secrets-e2e.sh` | `src/broker/gate.rs` except a call into the existing admit API | From a pane in A: `secret exec --cwd B`, `secret grant` for B, `secret read` of B, and `pane new --cwd B` are `permission_denied` with no value. A human click on the read's pending allows one read. `PLEXI_E2E_SKIP_PANES` is not set. |
| P8 Grant seal and one store | F08 | #2718, retargeted onto alpha after P6 | `src/broker/seal.rs`, `src/broker/host_key.rs`, `src/app/permissions.rs`, `scripts/permissions-seal-e2e.sh`, `scripts/app-share-e2e.sh` | `src/agent/*`, skill, editor | Forged `grants.toml` quarantines and reverts to ask. A deleted audit line denies the next call and files Needs you. `cat` of the profile and `secret get` do not print the MAC key. `permissions list` shows every app capability. Deleting `permissions.toml` changes nothing. |
| P9 Leads, queue, command view | F05, F06, F09, F10, F11 | #2715 → #2716 → #2717 on #2706. Close or abandon #2707 rather than merging both. | `src/agent/leads.rs`, `src/agent/queue.rs`, `src/agent/heads.rs`, `src/app/command_view_app.rs`, lead and command-view arms in `src/app/lifecycle.rs` | `src/broker/gate.rs`, skill, `text_editor_app.rs`. Leave the KeyPane and ClickPane arms to P5. | Two heads in one context, separate transcripts. `assistant send --head` and `command-view send` each produce a mock-model turn. `command-view cancel` stops the run. Assign with no pane reaches a terminal state. Restart reports the queued task or `outcome_unknown`. `agent head create` from a pane cannot attach a grant the pane does not hold. `command-view resolve` does not grant. |
| P10 Phone | F13, F18 | #2708, rebased after P5 and P6 | `clients/`, `services/relay/`, phone arms in `src/app/app_call.rs` | `src/agent/heads.rs`, editor, skill | `services/relay/e2e_installed.sh`: pair with a desktop-confirmed code, phone PONG does not cross a desktop turn, irreversible approve returns "waiting on desktop", relay logs have no canary body. |
| P11 Change sets | F19 | new alpha port of #2695, not the spike PR | `src/app/text_editor_app.rs`, `src/host/changes.rs`, `scripts/change-sets-e2e.sh` | gate, heads, relay, skill | Diff is visible, disk unchanged until a human click. Revert restores the agent hunk. A conflicting edit is stale. Twenty `pane key` calls return in under 1s and do not accept. |
| P12 Command-view docs | #2707 CI | whichever command-view PR actually merges (P9) | `website/src/content/docs/pgap.md` only, generated | do not hand-edit; do not run `just gen-capability-docs` (that recipe commits and pushes) | `check-capability-docs` is green. |
| P13 Acceptance harness | — | #2721 | `scripts/v1-acceptance.sh` only | product Rust | The script's summary lists each contract item as PASS, VERIFIED-VIA-BYPASS, or NOT-LANDED against the installed binary. It does not itself implement the items. |

P1, P2, and P12 can run beside the others. P3 touches only scripts, and only one agent per script. P5, P6, and P8 all touch `src/broker/gate.rs`: finish them in that order, one agent at a time. P9 and P10 both touch `src/app/lifecycle.rs`: P9 owns the lead and command-view arms; P10 rebases after. P4 is already stacked on P5's branch (#2722 on #2720); do not retarget it onto alpha until P5 has absorbed needs-you (F13).

## Not a work package

- House agent canned replies (F21), connector OAuth, Touch ID, XML/video editors, background pane layout, `cmd:` file handlers, marketplace upload (F20, F22).
- Change sets over MCP and lead-to-lead messaging. Ian marked both out.
- Container isolation. D1 is tamper-evident for same-user processes.
- In-memory run tokens on #2706. Restart rejection is the V1-09 step 4 requirement.
- Duplicate chess panes. Alpha `src/app/app_call.rs:202` already returns `ambiguous_instance`. #2711 is the installed check, not a new stub.
- Stable-channel gates in `src/release.rs` that hide Assistant and MCP. D0 keeps stable on the 0.3 host until a deliberate gate flip.

## Running work named in QUEUE.md

QUEUE.md (01:30–03:05 ET the same day) already assigned these. This inventory did not poll the agents.

| QUEUE id | PR |
|---|---|
| W15, W16, W2, W7 | #2709, #2714, #2720, #2722 |
| W3 | #2710 |
| W1 | #2719 on #2705 |
| W9 | #2713 on #2704 |
| W4, W10 | #2718 on #2700 |
| W8, W13 | #2711, #2712 |
| W11 | #2708 |
| W5, W6, W12 | #2715, #2716, #2717 |
| acceptance harness | #2721 |
| chess, cloud, needs-you, secrets, agents API, display-only command view | #2702, #2703, #2704, #2705, #2706, #2707 |

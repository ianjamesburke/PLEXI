# Agent sandbox profile per harness (v1.1)

**Stint:** 1273 (this design). Implementation: 0788 Part 3, 0805, 0577 and the follow-ups at the end.
**Status:** active. Design proposal awaiting Ian's rulings; no code.

**Problem.** An agent Plexi launches runs with the person's full macOS authority. Ian's ruling (2026-11-12) is that this ends in v1.1. Today only Codex gets a vendor sandbox, with network on; Claude Code and Pi get none (`spawn_command` in `HARNESS_ADAPTERS`, `src/cli/agent.rs`).

## Evidence rules

- **Observed** means read from the installed CLI on this Mac (macOS 14.5, `sw_vers`) on 2026-11-12: `--version`, `--help`, or the installed package files. No sandbox experiment was run for this doc.
- Sandbox behavior observed earlier is quoted from stint 0788 "Verify-first result" and "Part 3" (2026-10-14, older CLI versions) and labelled with that version. Re-qualifying it on current versions is stint 0805's job.
- macOS claims cite the man pages shipped with macOS: `sandbox(7)`, `sandbox_init(3)`, `sandbox-exec(1)`. Anything they do not state is marked UNKNOWN.

## What Plexi launches today (alpha)

- **Codex:** `--sandbox workspace-write -c sandbox_workspace_write.network_access=true`. Agent `settings.toml` flags may not set `sandbox*` or approval keys ("reserved for Plexi sandbox authority").
- **Claude Code:** no sandbox settings.
- **Pi:** `--no-extensions --no-mcp`, no sandbox.
- **agy and `script`:** no sandbox. Outside this stint's three rows; covered by decision 2.
- A harness the person starts by typing its name in an ordinary terminal is not a Plexi launch and gets no profile. 0788 Part 3 records a shell-function wrapper plan for that case.

## Harness table

| | Claude Code | Codex | Pi |
|---|---|---|---|
| Installed | 2.1.322 (`claude --version`) | codex-cli 0.167.0 (`codex --version`) | 1.4.0, `@earendil-works/pi-coding-agent` (`pi --version`) |
| Vendor mechanism | Settings object passed with `--settings`: `sandbox.enabled`, `failIfUnavailable`, `allowUnsandboxedCommands:false`; enforced even with `--dangerously-skip-permissions`; `sandbox.network.allowUnixSockets:[<realpath>]` admits one socket, the `/tmp` symlink form fails (0788, 2.1.300). `--help` lists no sandbox flag. | `-s read-only \| workspace-write \| danger-full-access`, "the sandbox policy to use when executing model-generated shell commands" (`--help`). `codex sandbox --help`: "Full command args to run under seatbelt". `-c sandbox_workspace_write.network_access=false` accepted (0788, 0.162.1). | None. The package's `docs/security.md` lists "lack of a built-in sandbox" as outside its boundary; `docs/containerization.md` points to Docker Sandboxes or OpenShell. No `sandbox-exec`, seatbelt, landlock or bwrap string in `dist/`. `--tools`/`--exclude-tools` select tools; they are not OS enforcement. Codemode scripts run in QuickJS, which is not the bash tool. |
| What the sandbox covers | The Bash tool's child commands. Hooks run outside it (0788). | Model-generated shell commands (`--help`). Whether `apply_patch` writes are bounded: UNVERIFIED. | Nothing. |
| Proposed write scope | Agent working root and a per-pane temp dir | `workspace-write` (working root plus vendor temp roots) | Same as Claude, enforced by a Plexi profile around the whole `pi` process |
| Proposed network scope | Commands: none except the host socket. Model traffic is from the unsandboxed `claude` process. Whether an empty domain list denies all TCP: UNVERIFIED. | Commands: off. 0788 observed network-off also blocks the host socket; a per-socket allowance is UNKNOWN (decision 4). | The `pi` process needs its provider hosts; everything else denied. Host-list enforcement by seatbelt: UNVERIFIED. |
| Host socket | Allowed by realpath; the CLI works once 0805 makes socket verbs run on a read-only profile (0788 observed the profile init lock failing). | Blocked with network off (0788). | Allowed by path in the Plexi profile; needs 0805. |
| Stays unprotected | The `claude` process, its in-process file tools (scope UNVERIFIED), hooks, and its environment: a sandboxed command read the parent's env through `sysctl(KERN_PROCARGS2)` in mask and deny modes (0788). Vendor default read scope: UNVERIFIED. | The `codex` process and its environment (KERN_PROCARGS2 read not tested on Codex: UNVERIFIED, assume same). Everything reachable over the network while it is on. | Its own provider credential, which lives under its agent directory and must be readable inside the profile (file not opened for this doc). |
| Sandbox cannot start | `failIfUnavailable:true`: the harness refuses to run (0788). Plexi reports a failed spawn and never retries unsandboxed. | UNVERIFIED: `--help` shows no fail-if-unavailable option, so silent fallback cannot be ruled out. 0805 records it before this row ships. | The Plexi launcher exits non-zero with a named error before `exec`; the host shows a failed spawn. |

## Why Plexi cannot wrap Claude Code or Codex in its own seatbelt

- `sandbox(7)`: "New processes inherit the sandbox of their parent." A Plexi profile on the pane's process is therefore already in force when `claude` or `codex` applies its own profile to a child command.
- 0788 observed on Ian's Mac (2026-10-13) that the nested `sandbox_apply` fails. Apple documentation on applying a second profile inside a sandboxed process: UNKNOWN; `sandbox_init(3)` and `sandbox-exec(1)` do not cover it. With `failIfUnavailable` the harness would refuse to start; without it the vendor sandbox would be lost.
- So for these two harnesses Plexi's only lever is the vendor setting, and the vendor's parent process stays outside every sandbox.
- Pi applies no profile, so a Plexi profile around the whole `pi` process is the only one in the tree and nesting never arises. It also covers more than either vendor does: the harness process, its file tools and its extensions are inside.
- `sandbox_init(3)` and `sandbox-exec(1)` are marked DEPRECATED, with Apple pointing to App Sandbox. `docs/assistant-authority-model.md` (stint 0554) accepts that deliberately: App Sandbox is a whole-bundle entitlement, not a per-child policy.

## Policy for a harness with no native sandbox

| Option | What | Why | Why not |
|---|---|---|---|
| A. Refuse | Plexi will not launch it as an agent | Honest; nothing to build | Pi is the default harness direction (0796); refusing it removes it |
| B. Label | Launch with an "Unsandboxed" badge | Cheap | A label is advice; it breaks the v1.1 ruling and trains people to click through |
| C. Plexi seatbelt | A Plexi launcher applies a profile compiled from a capability grant through the 0577 seam (`sandbox_init` in the child, then `exec pi`) | Bounds the whole harness; reuses the seam MCP servers need | Needs 0577 first; deprecated API; the provider credential is inside the box |

**Recommendation:** C for Pi; v1.1 does not ship until the Pi row is real. Any harness without a row (agy, `script`, future CLIs) gets A, unless the person opts that agent in through a click-only broker approval, in which case it gets B. The opt-in must be a sealed grant: `settings.toml` is writable by any same-user process, so a field there is not consent.

## Launch profile shape

One data row per harness, compiled into the host next to `HARNESS_ADAPTERS`, in the 0577 grant vocabulary (paths, network, socket). Never read from `settings.toml`; extend the existing Codex "reserved for Plexi sandbox authority" rule to Claude's `--settings` and to any Pi launcher argument.

- **Write:** the agent's working root and a per-pane temp dir. Never the home directory, shell startup files, other worktrees or the Plexi profile directory.
- **Network:** commands get none. Harness model traffic stays with the vendor process (Claude, Codex) or a listed provider host set (Pi).
- **Host socket:** allowed by realpath, read-only profile per 0805.
- **Environment:** no secrets (0788 Part 1, merged). `PLEXI_HOST_MCP_TOKEN` is still a pane-env bearer (audit PLAN, R1 "host MCP bearer", v1.1); a sandboxed command can read it from the parent.

## What stays open after the three rows ship

- **The socket is an escape.** Pane driving (`pane send`, `pane key`) stays open to any same-user caller until v1.1 (audit PLAN decision 4). A sandboxed agent that reaches the socket can type into an unsandboxed pane and run anything there. The rows are decorative until the host refuses operator verbs on another pane when the stamped origin (stint 1285) is a sandboxed pane.
- The Claude and Codex parent processes, their hooks and their environment.
- Reads outside the write scope (dotfiles, keys under `~`) unless the profile denies them; vendor read defaults are UNVERIFIED.
- Panes the person starts by typing a harness name.
- Any machine holding the signing key for the build it runs (audit PLAN, "Never claim").
- Linux and Windows: the 0577 seam stubs fail closed.

## User-visible contract

- Every agent pane header shows a badge set by the host from its own launch record, never from the pane: **Sandboxed** (harness, profile) or **Unsandboxed** (only after the sealed opt-in).
- Clicking the badge shows the profile: write roots, network, socket, and the "stays open" lines that apply to that harness.
- Widening: the agent asks through the broker; the person approves by click in the Plexi window (1264, 1271); the grant is sealed host-side; it takes effect when the pane relaunches and resumes its session. Live widening is not offered: once applied, a profile cannot be loosened (per 0554 doc; Apple documentation: UNKNOWN). Vendor settings reload at runtime: UNVERIFIED.
- Wording: "agents are sandboxed" stays banned (audit PLAN) until all three rows and the socket fix ship.

## Follow-up stints

| # | Work | Owner | Blocked by |
|---|---|---|---|
| 1 | Claude and Codex rows, reserved flags, fail-closed spawn | Exists: 0788 Part 3. Scope unchanged; link added. | 0805 |
| 2 | Read-only profile for socket verbs, Codex socket question, Codex fail-if-unavailable observation | Exists: 0805 | none |
| 3 | Capability-grant sandbox seam | Exists: 0577 (asks Ian to confirm v1.1) | none |
| 4 | Pi row: Plexi seatbelt launcher on the 0577 seam | Not filed: shape depends on decision 1 | 0577, 0805 |
| 5 | Host refuses operator verbs on other panes from a sandboxed origin | Not filed: depends on decision 3 | 1285 |
| 6 | Sandbox badge, profile sheet, click-only widen and relaunch | Not filed: depends on decisions 2 and 5 | 0788 Part 3, 1271 |
| - | Separate OS user or containers | Not needed for v1.1; recorded as options only | |

## Decisions for Ian

1. **Pi gets a Plexi-owned seatbelt (option C), and v1.1 waits for it.** Recommend yes. Why: Pi is the default harness and has no sandbox of its own. Cost if wrong: v1.1 slips by the 0577 seam.
2. **Harnesses with no row are refused as agents unless the person opts one in by click, which shows "Unsandboxed".** Recommend yes. Why: a silent default to full authority is what v1.1 removes. Cost if wrong: agy users lose agent launch until they click once.
3. **v1.1 is not done until a sandboxed pane cannot drive another pane.** Recommend yes. Why: otherwise every row is bypassed by one `pane send`. Cost if wrong: automation that drives sibling panes from a sandboxed agent needs a grant.
4. **Codex runs commands with network off; if 0805 finds no socket-only allowance, Codex agents lose the Plexi CLI from their shell rather than getting network back.** Recommend yes. Why: network on is an open exfiltration path. Cost if wrong: Codex workers must report through the host MCP endpoint instead of `plexi agent done`.
5. **Widening takes effect on relaunch, never live.** Recommend yes. Why: a profile cannot be loosened in place. Cost if wrong: one restart per approval.
6. **Move 0577 to v1.1.** Recommend yes. Why: decision 1 depends on its seam. Cost if wrong: MCP sandboxing work lands earlier than planned.

# Plexi V1: approved app-to-CLI execution

Status: active

Stint: 1171 (audit and contract); implementation stints to be scoped from its findings.

## Outcome

The V1 gate is approved app-to-CLI execution, a reusable capability for sandboxed apps. The separately installed stint board is its first proof consumer, not a bundled default app or a stint-specific host implementation. A person opens it in Plexi, views tasks grouped by priority, searches and reads details, and changes priority with keyboard or mouse. An agent reads the same change through stint; agent edits appear in the board without a translation step. Stint owns task semantics and files. The board owns presentation. Plexi owns the sandbox, execution boundary, consent, audit and app lifecycle.

## User flow

1. Start from a fresh Plexi install with no stint board preinstalled. In the stint repository, run the documented command that explains how to install the Plexi app; follow those instructions to install the matched CLI and separate board package, then open the app in the workspace through the existing app surface and palette. The exact command remains to be designed and tested for discoverability.
2. Plexi identifies the executable and workspace, presents the requested read and priority-change permissions through the existing approval surface, and supports a read-only board if mutation is denied. A remembered scope avoids a prompt on each move; revocation blocks further calls.
3. The board lists tasks and reads details through approved operations. A keyboard action or mouse priority menu requests a change based on the priority last displayed. The UI confirms only after stint reports success.
4. An outdated priority produces a conflict and refreshed state, with an explicit retry. An uncertain timeout is reconciled before retry. Agent changes refresh while the board is visible and immediately on refocus; choose the visible refresh interval by measurement. Audit reads without obscuring individual executions.
5. Incompatible CLI/app versions fail with clear update instructions. Changes to approved authority or executable identity require review and renewed approval according to the final contract.

## Build on what exists

Before choosing a transport, trace current callers and compare the descriptor-based CLI renderer, Python MCP connection, process supervision, permission broker, attention queue, package lifecycle and app CLI. Reuse existing code and contracts where their authority and lifecycle match. Record the chosen path, whether MCP should eventually consume this primitive or remain distinct, and why. Do not silently run arbitrary executables for discovery.

The CLI renderer currently sends commands to a linked terminal; it is not established as a structured request/result channel for sandboxed apps. The Python MCP connection offers a structured path to configured servers and is the nearest existing model, but a stint MCP server is not a settled requirement. The CLI descriptor's `plexi_app` and `capabilities` fields are parsed, while the guide's claimed custom-process launch has not been found in current callers. Trace them and delete or deliberately replace obsolete fields, consumers, documentation and tests; do not preserve unused compatibility. Historical CLI-native PGAP proposals predate the current sandbox and are not an implementation contract.

## Capability and trust contract

The app manifest requests the maximum scope; host-managed consent grants authority. Use the existing permission monitor, attention queue, revocation surface and sealed grant store. Python manifest membership alone is not sufficient consent for the new operation. Preserve existing once and remembered-grant semantics and label their duration truthfully.

The host identifies the app, workspace, approved operation definitions and executable. It checks the approved scope for every invocation. A guest must not choose an arbitrary executable, shell command, environment or working directory per call. Use typed validated arguments, bounded output, cancellation and process cleanup. Reapproval must prevent changed definitions or executable content from silently inheriting widened authority; choose an enforceable identity design during the existing-surface audit. Do not rely on a mutable executable path alone. Prevent duplicate dispatch after approval and log admission, denial and outcomes without exposing secrets.

Use the capability-grant vocabulary in stint 0577's binding platform-seam ruling, with path and network authority expressible for a future sandbox profile; do not introduce a competing authority vocabulary. Typed operation constraints and executable identity may add necessary fields to that grant and must not be mistaken for OS confinement.

For V1, 0577 records consent, audit and review as the interim security posture. Stint 0751 gates the arbitrary configured MCP server spawn path; `mcp.client` remains beta-gated. This proposal admits only approved typed operations on a pinned executable, but the native executable still runs with ambient OS authority. Its approval and review must disclose that fact; pinning does not sandbox it. An unrestricted coding agent can invoke it directly or alter files it owns. The grant protects the Plexi app boundary, not the whole user account. Approval accepts trusted physical keyboard or mouse input in the Plexi window through the Attention queue. Reject agent-originated socket, CLI and synthetic input. Verify how the host establishes trusted origin; do not claim synthetic input is blocked without evidence. A guest-reported human gesture is not proof of human consent. AI chat delegation and gesture-based authority are outside this slice.

## Data and concurrency

Use stint's existing `list --json`, `show --json` and `set --priority` where their contracts fit. Add an expected-current-priority guard, including an explicit representation for an unset priority. The board does not read task files directly or maintain a second task store.

All cooperating stint commands that can read and rewrite a task must use the same lock around read, change and atomic replacement. A lock only on the board mutation cannot prevent another command from overwriting it. Atomic replacement avoids partial files but does not by itself prevent lost updates. Define and test interactions with add, claim, moves/renames and multi-task operations. Advisory locks cannot control arbitrary editors or old binaries; document that boundary. Do not introduce a general revision system unless the audit demonstrates it is needed.

## Distribution

Keep the app in `stint/packages/plexi-stint` beside `packages/pi-stint`. `Cargo.toml` already excludes `packages`; verify `cargo package --list` excludes the board and generated packages and that the crate builds independently without Plexi or Python. Produce a separate `.plexipkg` from the corresponding stint release. Document installation of both artifacts and compatibility requirements. Verify replacement and permission changes with the existing installer; scope a concrete installer fix if needed.

Binary embedding, automatic app updates, GitHub subpath installation, marketplace work, drag-and-drop, within-column ordering, status transitions, and general task editing are not V1 requirements. Build initially behind an experiment and graduate based on actual use and V1 qualification.

## V1 acceptance

Qualify the capability independently with a test consumer: denial with zero execution, once and remembered consent, revocation, executable replacement, changed operations, workspace substitution, tampered grants, bounded process failures, restart, and the normal signed macOS Keychain path. A PR-channel file-backed key is not proof of normal Keychain behavior.

Qualify the board from a fresh Plexi install: discover and follow the command in the stint repo to install the separate app; display and search tasks; change priorities by keyboard and mouse; demonstrate conflict and retry, human change followed by agent CLI read, agent change followed by board refresh, and incompatible-version handling. Inspect actual rendered board pixels and exercise live behavior. Record source, fixture, installed and Ian Hand acceptance separately. Follow `src/testing/TESTING.md` and existing release gates; no-frame screenshots remain an evidence gap.

Click-only approval work in 0788 item 2 is a prerequisite for claiming secure consent; verify its current merged portions and remaining gaps before defining the dependency. Stint 1159 runs in parallel and is a prerequisite for the prescribed screenshot-based board acceptance. No-frame is an evidence gap, not a passed visual check.

## Proposed stint changes

These are scope proposals, not task state. Resolve the existing-surface audit before fixing exact implementation boundaries. Use the create-stint workflow, check duplicates and dependencies, then give the PRM its required `Stint:` line when filing.

| Repository | Proposal | Scope and dependency |
|---|---|---|
| Plexi | New: existing-surface audit and contract | After the 0788 item 2 prerequisite, trace and choose the CLI/app execution path; decide descriptor cleanup, authorization binding, structured results and installer replacement behavior. Produce the implementable PRM contract. |
| Plexi | New: approved app-to-CLI execution | Extend the chosen existing surfaces for scoped typed calls, execution, consent, audit and lifecycle; remove superseded zombie contracts. Depends on the contract. Split only where independent validation justifies it. |
| stint | New: safe task mutations | Expected-priority check and shared cooperating-writer locking with atomic replacement. Explicitly blocked by resolution of Plexi 1128's ownership of the dirty stint checkout, including `src/cmds.rs` and `src/repo.rs`; test competing CLI writers. |
| stint | Revise existing 0020 | Board and matched package in `packages/plexi-stint`; use existing JSON reads and `set`, include crate exclusion verification and installation instructions. Remove obsolete status-move, subpath-install and missing-JSON assumptions. Depends on the chosen Plexi execution path and safe stint mutation. Preserve the real 1128 ownership constraint until resolved. |
| Plexi | New or folded into board/bridge acceptance | End-to-end installed V1 qualification, including actual Keychain, update, human/agent handoff and Ian's Hand. Assign a distinct task only if it has a real independent owner and dependency. |

## Existing tasks to retire, revise or retain

- **Revise stint 0020**, not delete: it owns the actual board. Replace its obsolete design and dependencies with the scope above.
- **PLEXI 1167 and 1168:** neither is needed for this V1 workflow. Keep deferred for now; delete or supersede only after deciding that their broader subpath-install and update-check problems no longer warrant independent work. Do not close them merely because they are off the critical path.
- **PLEXI 1128:** retain its current ownership constraint and establish isolation before touching the stint checkout. It is not a board implementation task.
- **PLEXI 1129 and 1158:** retain their existing upgrade and authority work and their release evidence boundaries. Determine actual dependencies during scoping rather than inventing edges.
- **PLEXI 0788 item 2 and 1159:** approval security precedes the audit/contract and primitive; screenshot repair proceeds in parallel and gates installed screenshot acceptance. Existing task 0788 records some item 2 work as merged, so verify the exact residual before assuming every stated defect is current.
- **Old descriptor/native-app work:** retire or delete only after a caller and history audit identifies the exact obsolete code and tasks. No other task has been verified safe to remove.

## References

- `NORTH_STAR.md`, `TRUTH.md`, `docs/wasm-runtime.md`, `docs/AGENTS.md`, `src/testing/TESTING.md`
- `src/render/CLI_APP_CONTRACT.md`, `src/render/mcp_renderer_app.rs`, `registry/CLI_DESCRIPTOR_GUIDE.md`, `src/app/plexi_descriptor.rs`
- `src/host/wasm_python.rs`, `src/host/mcp_client.rs`, `sdk/python/plexi_sdk/effects.py`, `src/broker/`, `src/app/attention_queue.rs`
- Stint `Cargo.toml`, `src/cmds.rs`, `src/repo.rs`, `.stint/tasks/0020-feat-plexi-plexi-app-folder-with-board-app-list-json-and-move.md`

## Human checkpoint

Before V1 acceptance, present Ian with one independently validated installed candidate and a clean-profile walkthrough. Start with a fresh Plexi install without the board. From the stint repository, discover and run the documented installation guidance, install the app and matching CLI, open the board, approve using the keyboard, change a priority, and observe the change through the CLI. Then make an agent-side change and observe the board refresh. Show denial, revocation and conflict handling, and report exact candidate identity and remaining limits. Stop for Ian’s explicit acceptance; an automated tester cannot substitute for it. This checkpoint does not waive earlier security tests or independent installed verification.

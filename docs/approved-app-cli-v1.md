# Plexi V1: approved app-to-CLI execution

Status: active

Stint: 1171, 1174, 1175, 1176; the separately owned stint board and task-mutation work require coordination in the stint repository.

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

The CLI renderer currently sends commands to a linked terminal; it is not established as a structured request/result channel for sandboxed apps. The Python MCP connection offers a structured path to configured servers and is the nearest existing model, but a stint MCP server is not a settled requirement. The CLI descriptor's former `plexi_app` and `capabilities` fields had no launch or grant consumer and were removed in stint 1176 (an intentional breaking change with no compatibility shim); a descriptor that still sets them now fails to parse. Historical CLI-native PGAP proposals predate the current sandbox and are not an implementation contract.

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

Trusted desktop approval work in 0788 item 2 is a prerequisite for claiming secure consent; see the audited keyboard distinction below. Stint 1159 runs in parallel and is a prerequisite for the prescribed screenshot-based board acceptance. No-frame is an evidence gap, not a passed visual check.

## Execution contract

Use the existing CPython-WASM effect/result bridge and the host's `PermissionMonitor`; add a generic one-shot CLI operation effect, not a stint-specific host command, terminal input injection, or an MCP server requirement. An effect carries a request ID, a declared operation ID and typed values. The host obtains the app identity and workspace from the running pane and selects the approved executable; the guest cannot choose these per request. Completion returns the same request ID with a bounded structured result or typed failure. Host servicing must continue in `App::logic` and for inactive contexts; closing the app cancels or cleans up its work.

The package declares each operation's stable ID, input types and bounds, fixed executable and subcommand identity, fixed argument template, output format and intended read or mutation authority. The host constructs an argument array without a shell. Dynamic values occupy declared argument positions and are validated before admission and again before launch. A flag-like value must not be interpreted as an option: use validated types, explicit option values and end-of-options delimiters where supported; reject definitions whose CLI semantics cannot guarantee the mapping. A structured result contract, output size limit, process deadline and concurrency limit apply. No guest-supplied environment, executable, current directory or shell text is accepted. The host pins a canonical workspace and explicitly selects the executable after user review; a missing or mismatched executable fails without probing an unknown program.

An operation grant is distinct for read and mutation and binds the host-established app identity, app package content, canonical workspace, operation definition hash, resolved executable identity and executable content identity. The definition hash includes argument mapping, validation and output contract. Changing any binding fails closed and requires a new decision; do not trust a mutable path or manifest assertion. The implementation must define and test a race-resistant relationship between executable validation and launch; a hash followed by an unchecked path-based exec is not sufficient. Use the 0577 capability vocabulary for filesystem and network authority, and extend the existing broker's binding schema for operation identity rather than creating another permission store. This vocabulary describes potential future confinement; it does not confine the V1 child.

Use `PermissionMonitor` admission, correlated pending decisions, sealed grants, audit and the existing Permissions app. Explicitly support operation-scoped remembered grants without turning `args_unbound` into arbitrary argv authority. Once is consumed by exactly one admitted invocation; remembered grants are rechecked on every invocation. Denial, revocation, mutation of a binding, failed persistence and audit failure must fail closed before launch. The approval presentation names the app, workspace, executable, operation and ambient authority of the native program. It must not present an untrusted app's label as proof of what the executable can do. Tests must distinguish user consent to an operation from native OS confinement.

MCP remains a distinct protocol and its existing release gate remains in force. Its configured long-lived server has a different lifecycle and broader ambient authority than a one-shot operation; do not route it through the new path or promise future convergence without a separate threat review. Reuse the bounded channels and process-lifetime patterns in `src/host/mcp_client.rs` where appropriate. The current `--plexi` descriptor is a presentation and terminal-command contract, not an operation authorization schema; keep the generic renderer and do not treat its values as a grant.

## Audit findings and integration consequences

The host-rendered CLI visualizer uses `RunInLinkedTerminal` and has host privileges; it is not a sandboxed app request/result transport (`src/render/CLI_APP_CONTRACT.md`, `src/render/AGENTS.md`). Python MCP effects go through `LivePythonPane` to a configured, beta-gated `McpConnection`; `mcp_servers.toml` supplies its command and the process runs outside the guest sandbox (`sdk/python/AGENTS.md`, `src/host/wasm_python.rs`, `src/host/mcp_client.rs`). The Python `RequestCapability` path checks manifest membership without independently prompting (`docs/wasm-runtime.md`, `LivePythonPane::handle_capability_request`). These surfaces can inform the implementation but do not establish the requested grant contract.

The `plexi_app` and `capabilities` CLI descriptor fields were removed in stint 1176 together with their schema entries, parser tests, inspect output and guide text, because no launch or grant consumer existed. The parser denies unknown fields, so a descriptor that still sets either now fails loudly; no shim was kept. The descriptor renderer and MCP implementation are unchanged.

The package installer validates and stages a `.plexipkg` then delegates to the directory installer, which removes an existing destination and copies the replacement. The traced replacement path does not establish reapproval on changed operation authority. Bind grants to verified content at admission and test upgrades, interrupted replacement and restored app identity; fix the specific installer lifecycle needed for safe updates rather than assume capability diffs are already enforced (`src/cli/app.rs`, `src/app/package.rs`).

`approval_keyboard.rs` already handles desktop keyboard approval with an arm delay, fresh keypress checks and `synthetic_input_frame` rejection. The Attention decision overlay has a separate keyboard handler that maps Enter and number keys to `pick_decision_overlay_option`; the pointer selection checks `synthetic_input_frame`, while the baseline overlay keyboard handler does not. A focused harness test reproduced a real broker request being approved when the handler received a synthetic-marked keyboard frame. `raw_input_hook` marks Plexi pane-injected events synthetic, but the full pane-input route and installed behavior remain unverified. Require a failing-then-passing full input-route regression and zero grant/zero execution; do not claim this proves all operating-system injected input distinguishable. Verify trusted keyboard and mouse choices in the exact Attention queue flow. Existing 0788 notes record its earlier non-click refusal work as partly merged, so do not claim the whole item is unfinished or use a blanket dependency without identifying the residual.

## References

- `NORTH_STAR.md`, `TRUTH.md`, `docs/wasm-runtime.md`, `docs/AGENTS.md`, `src/testing/TESTING.md`
- `src/render/CLI_APP_CONTRACT.md`, `src/render/mcp_renderer_app.rs`, `registry/CLI_DESCRIPTOR_GUIDE.md`, `src/app/plexi_descriptor.rs`
- `src/host/wasm_python.rs`, `src/host/mcp_client.rs`, `sdk/python/plexi_sdk/effects.py`, `src/broker/`, `src/app/attention_queue.rs`
- Stint `Cargo.toml`, `src/cmds.rs`, `src/repo.rs`, `.stint/tasks/0020-feat-plexi-plexi-app-folder-with-board-app-list-json-and-move.md`

## Human checkpoint

Before V1 acceptance, present Ian with one independently validated installed candidate and a clean-profile walkthrough. Start with a fresh Plexi install without the board. From the stint repository, discover and run the documented installation guidance, install the app and matching CLI, open the board, approve using the keyboard, change a priority, and observe the change through the CLI. Then make an agent-side change and observe the board refresh. Show denial, revocation and conflict handling, and report exact candidate identity and remaining limits. Stop for Ian’s explicit acceptance; an automated tester cannot substitute for it. This checkpoint does not waive earlier security tests or independent installed verification.

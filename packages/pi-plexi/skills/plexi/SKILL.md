---
name: plexi
description: "Operate a live Plexi workspace from Pi: inspect contexts and panes, discover and call app tools with exact resource permissions, subscribe to app event streams, and coordinate creative questions or approvals through Needs you."
---

# Plexi Workspace Skill for Pi

When the target channel is known, use its `plexi-<channel>` shim for every CLI
command below (for example, `plexi-alpha agent --help`). Bare `plexi` inherits
the calling pane's routing and can target a different host. `PLEXI_PANE_ID`
identifies your pane; agent panes may not have `PLEXI_SOCKET`.

When the registered `plexi_*` tools are available, use them for workspace and
app operations. Print-mode workers may have extensions disabled; use the CLI
in that case. Start with `plexi agent --help` for the create/spawn flow:

1. **Orient first**:
   - Call `plexi_workspace_overview` to see the active context, sibling panes, running apps, and any pending `Needs you` items or durable continuations.
   - Use `plexi_pane_inspect` to read another pane's state or terminal output (`capture: true`).

2. **App tool discovery and invocation**:
   - Call `plexi_app_tools` to list all live app tools and their JSON schemas in the current context.
   - Call `plexi_app_call` to invoke an app tool:
     - Read-only tools (`read_only = true`, e.g. `clip.state`) execute immediately without prompting.
     - Mutating tools (`read_only = false`, e.g. `clip.crop`, `clip.move`) are gated by the host permission broker on the exact target resource (`clip_id`) and revision (`expected_revision`) with an idempotent `operation_id`.
     - Never attempt to approve your own permission request from inside the pane; only the human at the Plexi desktop can click **Allow once** / **Allow for this session**.

3. **Creative questions vs. permission approvals (`Needs you`)**:
   - Use `plexi_needs_you_ask` when you need a creative or product choice from the human (e.g. choosing a crop preset `16:9` vs `9:16` vs `1:1` for a clip, with `choices` and `preview_refs`).
   - Creative answers settle the `Question` record and record the chosen answer on the durable continuation record, but **never** mint a `GrantRecord` or bypass tool permission gates.
   - After a creative question is answered, re-read the target resource state (`clip.state`) or pass `expected_revision` and `operation_id` when calling the mutating tool so a stale revision is detected cleanly (`stale_revision`) and never corrupts user changes.

4. **Session reattachment & continuation reconciliation**:
   - On startup or after a restart, call `plexi_needs_you_reconcile` (or `/plexi-reconcile`) to inspect open and resolved `Needs you` items and `ContinuationRecord`s for this session/pane.
   - If a continuation is already `completed` with a receipt, do not repeat the mutation.

5. **User commands (`plexi_run` / `plexi run`)**:
   - Call `plexi_run` with no `command` (or run `plexi run --json`) to list merged workspace + global commands (`[{name, scope, description, run, secrets}]`, secret names only).
   - Call `plexi_run` with `command: "<name>"` and optional `args` (or run `plexi run <name> [args...]`) to execute a named workspace or global command.

6. **Agents (`plexi agent --help`)**:
   - Run `plexi agent list` first to see whether a matching agent already exists in the workspace or global tier.
   - If not, create one with `plexi agent create <slug> --cli <pi|claude|codex|agy> [--model <model>] --persona "<markdown>" [--result-schema '<json>'] [--ephemeral]` (defaults to workspace `.plexi/agents/<slug>/`; pass `--global` for reusable user-wide agents).
   - Inspect with `plexi agent show <slug>` before spawning if unsure what CLI, model, or command it uses.
   - Spawn with `plexi agent spawn <slug> --task "<brief>" [--subcontext <name>] [--ephemeral]` and collect the structured return with `plexi pane result wait <pane_id> --timeout 300`.

Spawn returns immediately with a pane id; it does not wait for completion.
Wait in bounded slices well under your tool harness call limit, for example
`plexi pane result wait <pane_id> --timeout 300`. Exit 0 means done and prints
the result. Exit 2 means the slice expired: report progress and re-run the same
wait. Inspect errors on exit 1. Never use one long wait or spawn a replacement
worker just because a slice expired.

A print-mode worker asks its parent with `plexi agent ask "<question>"`, then
blocks for the reply with `plexi pane message recv --timeout 300`. Exit 0 prints
a JSON array of messages; on exit 2, re-run the bounded wait. Exit 1 is an error.
The parent replies with `plexi pane message send <worker-pane> "<answer>"`.
Workers finish with `plexi agent done --result '<json>' --summary "<text>"`.

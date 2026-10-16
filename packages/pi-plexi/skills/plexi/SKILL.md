---
name: plexi
description: Operate a live Plexi workspace from Pi: inspect contexts and panes, discover and call app tools with exact resource permissions, subscribe to app event streams, and coordinate creative questions or approvals through Needs you.
---

# Plexi Workspace Skill for Pi

When running inside a Plexi terminal pane (`PLEXI_SOCKET` and `PLEXI_PANE_ID` are set), use the registered `plexi_*` tools first before falling back to raw shell commands:

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

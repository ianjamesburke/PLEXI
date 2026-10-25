# Qualification Record: Harness Worker Permission-Posture Enforcement

Stint: 0964  
Shipped Baseline: #2872  
Date: 2026-10-25  

## 1. Summary

This document qualifies the permission posture enforcement shipped in #2872 for harness worker agent panes. The implementation enforces an agent's `settings.toml` `[permissions]` table across host-brokered capabilities (`secret.get`, `secret.list`, `pane.send`, and `app call`).

## 2. Qualified Contracts and Verification Evidence

### 2.1 Deny, Ask, and Allow-Not-Authorization
- **Behavior:**
  - A tool listed in an agent's `deny` posture list or under a `default_posture = "deny"` is denied (`Decision::Deny`).
  - A tool listed in `ask` posture list or under `default_posture = "ask"` halts and awaits human desktop approval (`Decision::Ask`).
  - An `allow` list entry or `default_posture = "allow"` expresses agent preference, **not** user authorization. Requests still require a human grant or exact session approval (`Decision::Ask`).
- **Evidence:** Verified by `broker::tests::posture_lists_and_default_apply_in_order` and `app::secret_get::list_tests::worker_posture_deny_and_ask_apply_to_secret_get_and_pane_send_and_resist_mid_run_edits`.

### 2.2 Posture Snapshot Immutability
- **Behavior:**
  - When an agent worker pane is spawned, its `[permissions]` posture is resolved and snapshotted onto `pane.agent_permission_posture`.
  - Mutating `settings.toml` on disk after spawn does not widen or alter the live worker's posture.
- **Evidence:** Verified by `app::secret_get::list_tests::worker_posture_deny_and_ask_apply_to_secret_get_and_pane_send_and_resist_mid_run_edits`, where `settings.toml` was changed from `deny` to `allow` mid-run and subsequent requests remained denied.

### 2.3 Parent Narrowing
- **Behavior:**
  - Child workers cannot relax restrictions set on a parent agent (`child.restricted_by(&parent)`).
  - Inherited `deny` and `ask` rules are merged into the child's posture.
- **Evidence:** Verified by `broker::tests::child_posture_cannot_relax_parent_restrictions`, `broker::tests::child_posture_inherits_a_parents_default_ask`, and `app::secret_get::list_tests::child_worker_inherits_parent_posture_restrictions_and_subcontext_reuse_keeps_grants_distinct`.

### 2.4 Restore Behavior
- **Behavior:**
  - Worker permission posture is serialized in `SavedAgentSession` on workspace save.
  - When restored via Standby or workspace load, `pane_permission_posture` restores the snapshot and enforces broker gating on subsequent IPC requests.
- **Evidence:** Verified by `testing::harness_tests::saved_agent_pane_restores_permission_posture_and_enters_broker`.

### 2.5 Retained Audit Evidence Without Secret Values
- **Behavior:**
  - The permission monitor records audit facts (`AuditFact`) for admissions, denies, and uses.
  - Resource identifiers (e.g. `FOO`) and argument fingerprints are recorded, but raw secret values are never retained in audit records or written to disk.
- **Evidence:** Verified by `app::secret_get::list_tests::worker_posture_deny_and_ask_apply_to_secret_get_and_pane_send_and_resist_mid_run_edits` and `workspace::secrets::folder::tests::folder_secrets_e2e_records_redacted_audit`.

### 2.6 Same-Name Actor / Subcontext Isolation and Tool-Scoped Grant Binding
- **Behavior:**
  - Grants bind `actor_id` (e.g. `pane:<id>`), `actor_scope`, `trust_origin`, and `context_id`.
  - Workers with the same agent name/role running in different subcontexts or different panes cannot reuse or inherit grants granted to another instance.
- **Evidence:** Verified by `broker::tests::tool_scoped_grant_keeps_actor_origin_and_context_bound`, `broker::tests::same_name_actor_subcontext_isolation`, and `app::secret_get::list_tests::child_worker_inherits_parent_posture_restrictions_and_subcontext_reuse_keeps_grants_distinct`.

### 2.7 Harness-Native Tool Boundary
- **Behavior:**
  - Harness-native tools (such as `pi` `bash`/`read`/`write`/`edit`, `claude -p`, `codex` workspace-write and network, and `agy`) execute inside the child harness process and remain outside Plexi's host broker gate.
  - Plexi does not grant blanket bypass or new authority over harness-native execution.
- **Evidence:** Documented in `skills/plexi-cli/SKILL.md`, `skills/plexi-cli/intro.md`, and `src/cli/AGENTS.md`.

## 3. Discovered Missing-Snapshot Fallback Boundary

During qualification testing, the missing-snapshot fallback was evaluated:
- For `secret.get`, `secret.list`, and `app call`, missing posture snapshot falls back to `Decision::Ask` (admission required), preventing bypass.
- For `SendToPane` in `src/app/lifecycle.rs`, `caller_and_posture` currently gates only if `pane_permission_posture(caller)` is `Some(...)`. When posture snapshot is missing on an agent pane, `SendToPane` proceeds without entering the broker gate.
- This reproduction is codified in `app::secret_get::list_tests::worker_missing_snapshot_fallback_and_boundary_qualification` and bounded in follow-up regression task (stint 1022).

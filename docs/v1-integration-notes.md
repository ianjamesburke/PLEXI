# V1 integration resolutions

Reference branch `plexi-v1-integration`. Do not merge it. Replay these choices when each PR rebases onto alpha and squash-merges, one at a time, in the order below.

Base is alpha `58c32232` (`Port Assistant token ledger and run tags onto alpha (#2710)`). That tree matches PR #2710 head `7708f965`. Do not merge #2710 again. #2707 is superseded by #2715–#2717; skip it.

The gate does not get weaker at any step. An agent-originated approval stays refused. A terminal resolve stays refused. A phone approve never grants a permission.

## Train

### #2702 `e876fa95`

Fast-forward. Chess harness isolation only.

### #2703 `85f58036`

Clean. Cloud accounts, retention, and the phone shell's token auth.

### #2704 `e215a118` onto #2703

Keep both CLIs: `LedgerCmd` and `NeedsYouCmd`. Help's AI group names `ledger` and `needs-you`. `main` imports both. The phone page keeps #2703 token auth and adds the Needs you list (`refreshNeeds`, `#needs-you`).

### #2713 `3965f094` onto #2704

Clean at this step. The Needs you journal lands as `<profile>/host/needs-you.json` with a file key `<profile>/host/seal.key`. That file key is retired when #2718 lands. See "One seal key" below. Do not keep `seal.key` on the final tree.

### #2712 `92b48fab` onto #2713

Keep `needs_you_store` and `integrity`. `NeedsYouKind` stays `Serialize` + `Deserialize`. On open, restore the queue, then file tamper through `file_profile_integrity`. `file_needs_you` refuses `NeedsYouKind::Integrity`, so a tamper marker must not call it.

### #2705 `d4690041` and #2719 `1e3d8c63`

Clean. Folder secrets, then the hardening that closes pane bypasses.

### #2706 `3d2db6f9` onto #2719

Keep Needs you IPC and the agents API. A ledger row carries alpha `client`, `kind: Option<RunKind>`, and `wall_ms`, and also `run_id`, `agent_id`, `client_ref`, and `parent_run_id`. An agent run copies `client_ref` onto `client`. `append` calls `append_result`, which returns `Result`, takes the ledger lock, and migrates null tags. A failed ledger line fails the agent spawn.

### #2715 `efc7e7aa` onto #2706

`SubmitAssistantTurn` carries `client`, `kind`, and `head`. A `head` routes to `agent::leads::submit_turn`. Otherwise the turn uses the headless assistant and `submit_tagged_turn`. The schema lists `client`, `kind`, and `head`. Shell tests use `await_pid_name`. Drop the duplicate `wait_for_pid_name`.

### #2716 `711350d6`

Clean. Headless queue.

### #2717 `8d27e06c` onto #2716

`CommandView { cmd, json, follow }` beside Needs you. Docs skip both `ledger` and `command-view`.

### #2709 `54baf078`

Clean. Human driver scripts.

### #2720 `22f92a5e` onto #2709

`needs-you list` stays. `needs-you resolve` and `assistant permission resolve` call `assistant_permission_cli`, and the host refuses. `command-view resolve` and `command-view allow` call `command_view_refused`. A desktop click still grants. Do not route a terminal resolve through `needs_you_cli` in a way that grants.

### #2722 `251346fb` onto #2720

`SkillCmd::Install { agent }` beside command view. `main` calls `skill_install_cli`. One `NeedsYouCmd` only.

### #2725 `9422c61d` onto #2722

`Commands::Ledger.cmd` is `Option<LedgerCmd>`. Bare `plexi ledger` is the per-client summary (`by`, `since`, and `json` all unset). A token count is a positive number or the word `unknown`. `append_result` still returns `Result` and logs `token_log` plus run ids. Later checkouts of `args.rs` and `main.rs` dropped this; put it back before moving on. Test: `bare_ledger_is_the_per_client_summary`.

### #2711 `33adfbbc` onto #2725

`SpawnPane` and `PaneLaunchSpec` carry `peer_ancestry` (host-stamped; a client cannot set it) and `force_new` (`app open --new`). Several instances and a bare app call return `ambiguous_instance` and do not pick. Approvals stay refused.

### #2700 `e7d9b7f4` then #2718 `485d0397`

#2700 is an ancestor of #2718. Merging the old #2700 tip onto this stack replays the whole permissions tree. The squash train still lands #2700 first, then the commits on #2718 that are not already in #2700.

The integration merged #2718's head and took permission-core files from the earlier resolution `753457e4` (grant store, `permissions_cli` list/reset/revoke/allow, host seal store). Then it put #2725's `Option<LedgerCmd>` back. `PermissionsCmd` is the grant-store surface. It does not call `assistant_permission_cli`. An agent `allow` or `reset` files Needs you and leaves the decision unchanged.

`Cargo.toml` declares `ring` once. The phone link and the folder-secret AES fallback share that dependency.

### One seal key (#2713 journal + #2718 host store)

One custody path. The journal HMAC is the host permission MAC (`seal::mac_key_bytes` / `host_key::MAC_ITEM`, account `permission-mac`). The last-MAC tip is `host_key` account `plexi:host:needs-you-journal-tip:` plus the sha256 of the canonical `<profile>/host` directory. The journal file stays mode `0600` under `<profile>/host` mode `0700`. Nothing writes `seal.key`.

`seal::existing_mac_key` reads the MAC and does not create it. `save` uses `mac_key_bytes`, which creates the MAC on first use, writes the tip, and deletes a leftover `seal.key`.

Load:

- Secret Service (or the platform host store) missing: untrusted. No plaintext fallback.
- Journal present, host MAC matches: trust it, and delete a leftover `seal.key`.
- Journal present, a legacy `seal.key` verifies it: rewrite with the host MAC (`save`), delete `seal.key`, then return the items.
- Legacy key does not verify, or the journal has no seal: delete `seal.key` and return untrusted. The caller quarantines the journal.
- Journal missing and the tip is nonzero: untrusted ("the host seal still remembers one"). A legacy file whose tip is nonzero is copied into the host tip, then the file is deleted, so the next load still sees the deletion.

`host_queue_directory_is_private` asserts the directory mode, the journal mode, the absence of `seal.key`, and that the MAC bytes are not inside the journal. `scripts/needs-you-persist-e2e.sh` asserts the same absence.

### #2708 `3df2cee4` onto #2718

Phone and relay files follow the previous integration resolution at `03164835`, which is #2708's phone stack plus the #2703 relay pieces that stack already had. `services/relay/relay.py` and `test_relay.py` are that resolution, not a raw take of #2708, because the raw PR drops relay behavior the earlier cloud PR added.

`needs-you resolve --from-phone` calls `needs_you_cli(..., from_phone: true)`. The host refuses an approval and a permission widen (`phone_blocked`: integrity, or approve when `phone_may_approve` is false). A phone turn's JSON has no approval, grant, or permission field. A terminal resolve without `--from-phone` still goes through `assistant_permission_cli` and is refused. Questions and blocked runs can still be answered.

`assistant_send_result` / `assistant_send_cli` take `text`, `head`, `request_id`, `pane_id`, `context_id`, `client`, `kind`, `conversation`, `join_desktop`, `status_for`. `SubmitAssistantTurn` has those fields. `submit_external_turn` takes `conversation_id`, `join_desktop`, and `status_for`. The default `submit_tagged_turn` forwards the three phone fields as none / false / none. The Assistant override joins the desktop transcript and passes `client` and `kind`.

Harness and send-test initializers include `head`, `client`, `kind`, `conversation_id`, `join_desktop`, and `status_for`. `NeedsYouKind::PermissionChange` is stored like a question: it must not carry a pending grant. The widen is applied from `run_tag` on a desktop resolve.

### #2724 `4d450b6a` onto #2708

Keep change sets and everything already on the stack.

- `Changes` command and `ChangesCmd` (including `profile`) sit beside `Secret`. `SkillCmd` stays its own enum.
- Help AI group: `ai`, `assistant`, `changes`, `command-view`, `ledger`, `needs-you`, `permissions`, `relay`.
- `AssistantCmd` has `Open` and `Tool`. `assistant_tool_cli` is added. The 10-argument `assistant_send_*` stays.
- `cli/mod.rs` exports both `changes::*` and `connector::*`, and both `assistant_tool_cli` and `needs_you_cli`.
- `handle_assistant_host_tool` keeps `host.introspect` and `host.permissions.*`, and adds `host.editors.list`.
- Schema and PGAP docs keep `open_assistant_head`, `command_view`, `agent_queue`, needs-you, and permissions, and add `assistant_host_tool`.
- Docs skip list adds `changes` and keeps `needs-you` and `relay`.

### #2721 `99bfa538`

Clean. `scripts/v1-acceptance.sh` lands on the integrated tree.

## After the train compiles

### Sealed audit text

`permission-audit.jsonl` is a chain of envelopes. The fact is a JSON string, so a line contains `\"decision\":\"use\"` rather than `"decision":"use"`. Tests that read the file (`agent::leads` grant and cancel, `host::changes` ask and commit) match through `seal::audit_count` / `seal::audit_contains`, which accept the raw fact or the escaped envelope. Do not unwrap the seal to make the old substring match.

### One integrity row for a tampered profile

`PermissionMonitor::open` inspects the stamp before load. An unsigned `grants.toml` is both a profile-change finding and a seal fault. When `profile_changed` is set, the `grants.toml` seal fault is not raised again; `file_profile_integrity` is the row a person acknowledges. Audit and other seal faults are still raised. `edited_profile_while_down_files_an_integrity_item` expects that one acknowledgement to clear the list.

### Folder-secret ask names the secret

`resource_of` for `secret.read` returns `ResourceScope::Path` and `{name}@{folder}`, the same id `grant_record` stores. The ask audit then contains the name. The value is not an audit field. Without this, a grant written as `{name}@{folder}` does not match the admission, and the ask fact has an empty `resource_id`.

### Clippy argument counts

Stacked signatures cross `clippy::too_many_arguments` only after the train is together. Allow it on `dispatch_model_turn`, `submit_external_turn_tagged`, `assistant_send_result`, `assistant_send_cli`, and `seed_window_root_pane`. The same allow is already used elsewhere in the host. Do not drop a field to get back under seven.

### Skill fence

`skills/plexi-cli/SKILL.md` closes the folder-secret `bash` block before the connector heading. An unclosed fence inverts later fences, and `skill_surface_matches_cli` then treats prose as a bare reference block. `skill_version` is `5.0.12`.

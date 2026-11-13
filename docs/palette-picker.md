Stint: 1390 (design), built by 1391, 1392, 1396, the Agents view stint
Status: active

# The palette: commands, then files, then agents

Release 0.6 Palette. Rulings: ledger rows 68, 69, 70, 75, and Ian's slice ruling of 2026-11-13. Checked against `alpha` on 2026-11-13.

## The journey

The things you do every day live on shortcuts. The palette is for everything past your shortcuts: Cmd+P opens the command list you know. Two things have no key at all, a file somewhere in this workspace and an agent somewhere in this machine, so Tab reaches them: Tab once for Files, Tab again for Agents.

## What 0.6 ships

| Stop | Shows | Order | Preview |
|---|---|---|---|
| Commands (Cmd+P) | Today's rows: host commands, user commands, contexts, plugins, notes, update ready. Plugin commands from stint 1151 join, labelled with their plugin. Files and agents never appear here. | as today (`score_candidate`, group order) | none |
| Files (Tab) | Files under the active context root, walked with `ScopedWalker` and its prune list | match score, then usage; empty query: usage, then newest modified | always: `text` or `image` |
| Agents (Tab again) | Every agent, waiting on you first | waiting on you, then last interaction | always: `text` |

Tab past Agents returns to Commands; Shift+Tab goes back. The query is kept across stops. Nothing here is configured: no palette table in `config.toml`, no pins, no per-view settings. The Explorer, the notes picker and Cmd+O are untouched in 0.6.

```
Commands (Cmd+P)                                 Files (Tab)
 [Commands] Files Agents                          Commands [Files] Agents
 > rel|                                           > pick|
 Reload configuration          Cmd+Shift+,        src/host/picker.rs     2m | //! Picker reducer
 plexi-release           context, 2 panes         docs/palette-picker.md 1h | Stint: 1390 (design)
 GitHub Tree               plugin: gh-tree        walking...  (rows above came from the cache)
 enter run   tab files   esc close                enter open   tab agents   esc close
Agents (Tab again)
 Commands Files [Agents]
 > |
 reviewer  waiting on you  2m | Ship 0.6 today?
 builder   working         5m | cargo test ...
 planner   not running        |
 enter focus   cmd+enter actions   esc close
```

## Prior art: Raycast

| Learning | Source | Applied |
|---|---|---|
| Ranking is learned from picks (frequency and recency, last after match quality), not configured; "Reset Ranking" forgets one item. | [Search Bar](https://manual.raycast.com/search-bar), [Action Panel](https://manual.raycast.com/action-panel) | Files rank by usage after match score; a "Reset ranking" action. |
| Enter runs the first action; one key lists every action for the selected item (Cmd+K there). | [Action Panel API](https://developers.raycast.com/api-reference/user-interface/action-panel), [Keyboard Shortcuts](https://manual.raycast.com/keyboard-shortcuts) | Cmd+Enter lists actions; Cmd+K already moves the selection here. |
| A list's detail area belongs to the list and shows for the selected item. | [List API](https://developers.raycast.com/api-reference/user-interface/list) | Preview is part of the stop, always on; no toggle. |
| Per-item settings are set in place, not in files. | [Command Aliases & Hotkeys](https://manual.raycast.com/command-aliases-and-hotkeys) | No palette config. |
| Ranking history is synced and cannot be turned off. | [Search Bar](https://manual.raycast.com/search-bar) | Usage history is channel-neutral user data. |

## Binding commands to keys

Not every palette command can be bound in `[keybindings]` today.
- Bindable: rows of `PaletteCommand::Host(Action)`, because `build_binding_table` turns each `KeyBindings` field into an `Action`, except `Action::OpenChangelog` (the "Changelog" row), which no `KeyBindings` field reaches. User commands bind through their own key in `commands.toml` (`KeyBindings::user_commands` to `Action::RunUserCommand`).
- Not bindable: the palette-only variants `OpenGlobalConfig`, `OpenWorkspaceConfig`, `StatePreset`, `EditCommands`, `SwitchCommandsScope`, `OpenAttention`, `SetupCli`, `UiUpcycle`, `CheckForUpdates` and `RestartToUpdate`. They have no `Action`.
- Context, plugin and note rows are destinations, not commands.

The gap is stint 1151's scope: one registry where every command has a stable id assignable in `[keybindings]`, with a coverage test that fails on an unregistered action. 1151 stays in 0.6 Palette, and its coverage test must name these variants and `OpenChangelog`. No new stint.

## The component underneath

Built properly once, so the Explorer, Notes and plugins can move onto it later without a rewrite.

The picker is a pure reducer in a new `src/host/picker` module with no egui types. `HostModel` holds one `PickerState` per `PickerId` (`Palette` in 0.6; `Pane(pane_id)` and `PluginNode(pane_id, node_id)` are reserved for Later), reached through `HostAction::Picker(id, PickerCommand)` and answered with `HostEffect::Picker(id, PickerEffect)`. Today the palette is `PlexiApp.palette_*` fields mutated inside `draw_command_palette`, and `command_palette_handle_key` is an empty stub. `PickerState`: the stop list and active stop; one `query`; `step` (`Results`, or `Actions` with the row's action list); ranked rows; `selected` as a row key, never an index; a query generation; loading and preview status.

| Commands in | Effects out |
|---|---|
| `Open(stop)`, `Close`, `NextStop`, `PrevStop`, `SetStop` (chip click) | `Query { stop, text, generation }` for the source runner |
| `QueryEdited(text)`, sent only by the text field | `LoadPreview(row key)` |
| `Move(delta)`, `Hover(row key)`, `ShowActions`, `Accept(action id)`, `Back` | `Activate { row key, action id }`, `Closed`, `Notice(text)` |
| `Rows { generation, rows, done }`, `Preview { row key, body }`, `SourceFailed` | |

Rules the reducer enforces:
- No key command edits `query`. Arrow, Enter, Tab and Esc never append or pop characters. The field is the default text surface (`register_default_text_surface`). In the `Actions` step the field filters the action list and the results query is kept behind it.
- Selection follows the row key across re-ranks. If the selected row disappears, nothing is selected until the person moves (today's `reconcile_palette_selection` rule).
- `Rows` from an older generation are dropped. Switching stops keeps the query and re-emits `Query`. A `Preview` for a row no longer selected is dropped.
- Actions never run in the reducer. `Activate` is resolved outside the model by the stop's source.

A stop is data defined in code: id, chip label, source ids, ranking preset, whether it carries a preview, placeholder. A source is host code outside the model that answers `Query` with rows. Commands and Agents answer in the same `App::logic` pass. Files answer from a background worker drained in `App::logic`; cached rows show at once and `done` clears the walking line.

A row carries: `key` (`<source id>:<stable id>`, split on the first `:`), title, secondary text, chips, search text, score from `search::score_candidate`, modified time, group, an optional unavailable reason, a preview reference, and actions (id, label, optional chord; the first is Enter). Unavailable rows stay listed, show the reason and refuse Enter. A source may return at most `MAX_SEARCH_HITS` rows per query; a larger answer is refused with a typed error and a visible notice. Rows, keys, action ids, preset names and preview kind names hold no host paths, egui types or host enums, so 1395 can carry them across WIT unchanged.

`src/ui/picker.rs` has painters that read a `PickerState` and return `PickerCommand`s: `Field` (chips, `TextField`, action list), `Results` (`ListRow`s with group headers, scroll to selection, hover and click) and `Preview`. Keys are translated in the focus owner's `handle_key`, never in a draw function. The palette shell stays `ModalShell::centered` with `FocusKind::CommandPalette`; opening keeps the invoking pane and selection as the command target.

## Files

- One index per context root, kept by the host and refreshed in the background when the palette opens. A file row's stable id is its canonical absolute path. The Explorer keeps its own `SearchSession` until it moves onto the picker.
- Usage: every `Activate` records the row key. With a query, usage breaks ties after match score; with an empty query it is the order, then newest modified. "Reset ranking" removes one row's entry.
- Usage lives in one file, `palette-usage.json` in `shared_dir()` (`~/.plexi`, channel-neutral, as `src/host/AGENTS.md` requires of user data), holding row key, pick count and last pick time; entries unused for `USAGE_FORGET_AFTER` drop out. Only `src/host/picker` reads or writes it, off the UI thread, debounced, by atomic replace. Two channels running at once can overwrite each other's latest picks; that only changes order.

## Preview

The source sets each row's preview reference: a kind and a target only it understands. The `Preview` painter draws by kind and never guesses from a file name. One classifier decides a file's kind; the Explorer adopts it when it moves (Later). A row with no supported kind shows one dim line: the source's reason, else "No preview".

| Kind | Shown | Limits |
|---|---|---|
| `text` | wrapped monospace, scrollable; Markdown as source text, as the notes picker shows it today | `TEXT_PREVIEW_MAX_BYTES`, then a "truncated" line (`read_text_preview`). A NUL byte means "Binary file". |
| `image` | fitted to the preview half, aspect kept | decoded and downscaled off the UI thread to the longest edge `ensure_image_preview` uses today, made a named constant. Over `PREVIEW_IMAGE_MAX_BYTES`: "Too large to preview". |

- `LoadPreview` goes to the row's source, which reads on a preview worker, never the UI thread. Results drain in `App::logic`; an image texture is uploaded there too. Agent previews are already in memory and need no worker.
- While loading, the half shows the row title and a dim "Loading". Past `PREVIEW_TIMEOUT`: "Preview timed out". A failed read: "Could not read: <error>", logged with the path at `warn`. None of these blocks the results.
- Below `MODAL_WIDTH_MD` the preview half is dropped, as `draw_notes_picker` does today.

Stint 1116 (0.8 Writing Desk, images in the notes picker preview) should load its images through this `image` loader rather than decode on the UI thread.

## Agents

One stop over the roster the Agents pane already builds; no second roster.

| Field | Comes from |
|---|---|
| Rows | `PlexiApp::agent_inputs`: every live agent pane in every context, `context_name` as secondary text. Named definitions with no pane (`AgentRegistry`) show as "not running" through the 1396 resolver. Closed runs in `fleet_history` stay in the Agents pane. |
| Name | `AgentRow.role` (`resolve_agent_label`); search text also holds the pane name, harness and task title. |
| State | `DisplayState` with `AgentRow.needs_you`: waiting on you (`Blocked` with `needs_you`), blocked, working, idle, done (`DoneUnseen`), failed (`Failed`, `Stalled`, `ExitedUnreported`), not running. `needs_you` already folds in `PermissionMonitor::open_needs_you`. |
| Latest message and preview | `BlockedOn::label` when waiting, else the newest `AgentEvent`, else `tail` (`capture_tail_preview`). |
| Time | `format_age` of the last interaction (see Missing). |

Actions, listed by Cmd+Enter:
- **Focus** (Enter): focus the pane and `mark_agent_seen`; a not-running row runs the 1396 `agent open` resolver.
- **Answer** (first while waiting on you): opens the Decision Overlay through `open_decision_overlay_for_ask`. The palette never resolves a permission itself.
- **New chat**: stint 1397.
- **Close pane**: the normal close path.

Missing, built by the Agents view stint:
- **Last interaction time.** `fleet_since` is the last agent state change, not the last time the person typed into, focused or answered the agent. Add a per-pane `last_interaction_at` set on those three events and persisted like `fleet_since`; without it a busy agent sorts first.
- **Snapshot.** `agent_inputs` loads `PermissionMonitor` from disk on every call. The source answers from a snapshot refreshed when agent state or the needs-you set changes.
- **Stop.** No typed action stops a turn and keeps the session; Close pane ends it. A harness-aware stop needs its own stint and is not in 0.6.

## Keyboard map

Chords checked in `KeyBindings::default`.

| Key | Results | Action list |
|---|---|---|
| Cmd+P | open on Commands; close | close |
| Tab, Shift+Tab | next stop, previous stop | none |
| Up, Down, Cmd+K, Cmd+J | move selection | move in the list |
| Enter | first action | run the action |
| Cmd+Enter | list the selected row's actions | none |
| Esc | close | back to results, query and selection kept |

`/` and `>` are ordinary characters; no prefix switches a stop. Cmd+Enter is `zoom_in` among host bindings; the modal wins because `poll_actions` skips `Normal` bindings while an overlay is open.

## Deleted

Stint 1391: `PaletteEntry::Agent`, `PaletteAction`, `action_for_palette_entry`, `sort_palette_entries`, `reconcile_palette_selection`, the key block in `draw_command_palette`, `command_palette_handle_key`, and the `palette_query`, `palette_selected`, `palette_selected_key` and `palette_scroll_reset` fields. The Agents view stint: the `[agents] palette_agent_results` key, from `src/config/mod.rs`, `scripts/default-config.toml` and `docs/CONFIG.md`. Stint 1151: `PaletteCommand`, `PaletteCommandEntry`, `PALETTE_COMMANDS`, `run_palette_selection`, `run_palette_command`.

## Build order

| Stint | Lands | Tests that must pass |
|---|---|---|
| 1396 named agent launch (first) | One `agent open` resolver (focus if running, else resume last chat, else new) shared by CLI and the Agents stop | `named_agent_focuses_when_running`, `named_agent_resumes_last_chat_when_stopped`, `named_agent_starts_new_when_never_run`, `cli_agent_open_matches_palette` |
| 1391 picker core | Reducer, painters, Commands stop on the component, Tab between stops | `tab_cycles_stops_and_shift_tab_goes_back`, `query_survives_stop_switch`, `no_key_command_edits_the_query`, `selection_follows_row_key_and_clears_on_removal`, `stale_generation_rows_are_dropped`, `mouse_and_enter_resolve_the_same_action`, `commands_stop_never_lists_files_or_agents`, `ui_tests::palette_typing_abc_reads_abc`, `ui_tests::palette_commands_stop` |
| 1151 command registry | Every command bindable; plugin commands join Commands with their plugin label | its own done-when, plus `every_palette_command_is_bindable` naming the variants above |
| 1392 files | File index, Files stop, usage ranking, `Preview` painter and worker with `text` and `image` | `file_walk_never_runs_on_the_ui_thread`, `palette_opens_a_file_with_no_explorer_open`, `files_stay_inside_the_active_context_root`, `cached_rows_show_before_the_walk_finishes`, `empty_query_orders_files_by_usage`, `usage_breaks_ties_after_score`, `reset_ranking_forgets_the_row`, `usage_file_is_channel_neutral`, `preview_never_loads_on_the_ui_thread`, `preview_for_a_moved_selection_is_dropped`, `oversized_image_shows_no_preview_with_reason`, `oversized_source_answer_is_refused`, `ui_tests::palette_files_stop_with_preview` |
| Agents view (new; after 1391 and 1396) | Agents stop, `attention` order, `last_interaction_at`, snapshot source, action list, Focus, Answer, Close pane | `agents_waiting_on_you_sort_first`, `agents_sort_by_last_interaction_not_state_change`, `answer_opens_the_decision_overlay_and_never_resolves_in_the_palette`, `cmd_enter_lists_actions_and_keeps_the_query`, `agents_source_does_not_read_disk_per_query`, `ui_tests::palette_agents_stop_with_preview` |
| 1397 agent memory and New chat | "New chat" as an Agents action and a command | `new_chat_archives_the_current_chat`, `archived_chat_reopens_from_history`, `new_chat_reads_the_same_memory_folder` |

## Later

Each waits on one trigger: Ian's daily use of this slice says the palette is good enough to replace the surface in question.
- **Explorer on the picker** (was stint 1394): docked picker in a right split, typing always filters, chord claim, `intent.rs` deleted.
- **Notes on the picker**: a Notes stop replaces the notes picker modal and Cmd+O lands on it.
- **Plugins** (stint 1395, 0.7 Workbench): a public Picker node and plugin-contributed stops, using the row seam above.
- **Inline fields** (stint 1393): commands that take input collect it under the field; needed once the first such command or the Explorer arrives.

## Decisions for Ian

| # | Decision | Options | My pick |
|---|---|---|---|
| 1 | Inline fields (1393) leave 0.6 Palette. No command in this slice needs typed input, but 1151's done-when says an argument-bearing command reaches "the same picker/form". | A. Move 1393 to Later; 1151 records argument schemas, and argument-bearing commands keep their current entry UI until 1393 lands. B. Keep 1393 in 0.6. | A |

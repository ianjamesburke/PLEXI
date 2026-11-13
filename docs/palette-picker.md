Stint: 1390 (design), built by 1391, 1392, 1393, 1394
Status: active

# One picker: the palette, the Explorer box and the plugin Picker

Release 0.6 Palette. Rulings: ledger rows 42, 68, 69, 70, 75. Checked against `alpha` on 2026-11-13.

## How it fits together

One field takes the person's text. Tab and Shift+Tab move between a fixed set of views: Everything, Agents, Files, Notes, Commands, then any installed plugin's views. Sources answer the query with rows, ranked by match and then by what the person picks most and most recently. Enter runs the selected row's first action; Cmd+Enter lists all of its actions. An action that needs input opens inline fields under the same field without losing the query. Views that carry a preview show it for the selected row whenever the row has a supported kind. Nothing about this is set in a config file. One reducer and one set of painters serve three hosts: the palette modal (Cmd+P, Cmd+Shift+P, Cmd+O), the Explorer docked in a right split (Cmd+E), and a plugin's Picker node.

## Prior art: Raycast

| Learning | Source | Applied here |
|---|---|---|
| Root search ranks by alias, then title fuzzy score, then subtitle and keywords, with frecency (frequency and recency of picks) last. Ranking is learned, not configured. | [Search Bar](https://manual.raycast.com/search-bar) | Match score first, usage last; the empty query is ordered by usage. No ranking settings. |
| "Reset Ranking" clears learned data for one item from its action panel. | [Action Panel](https://manual.raycast.com/action-panel) | A "Reset ranking" action on any row with usage. |
| Favorites stay at the top of root search when the bar is empty, added with Cmd+F. | [Search Bar](https://manual.raycast.com/search-bar), [Keyboard Shortcuts](https://manual.raycast.com/keyboard-shortcuts) | Not in 0.6; see Decisions. Usage ranking covers the same need with no new verb. |
| Per-item settings (alias, hotkey, disable) are set in place from the action panel; a settings screen mirrors them. | [Command Aliases & Hotkeys](https://manual.raycast.com/command-aliases-and-hotkeys), [Action Panel](https://manual.raycast.com/action-panel) | Anything per row is an action, never a config key. Aliases and hotkeys are not in 0.6. |
| Enter runs the first action; Cmd+K opens the action panel listing every action for the selected item. | [Action Panel API](https://developers.raycast.com/api-reference/user-interface/action-panel), [Keyboard Shortcuts](https://manual.raycast.com/keyboard-shortcuts) | Enter runs the first action. The action list opens on Cmd+Enter, because Cmd+K moves the selection here. |
| A list's detail area is a property of the list, shown for the selected item; the documented example toggles it with an ordinary action. | [List API](https://developers.raycast.com/api-reference/user-interface/list) | Preview is part of the view, fixed in code. No persisted toggle. |
| Quick Look on Cmd+Y previews a file without opening it. | [Keyboard Shortcuts](https://manual.raycast.com/keyboard-shortcuts), [File Search](https://manual.raycast.com/file-search) | Explorer Quick Look moves from Space to Cmd+Y. |
| Extensions declare commands in their manifest and appear in root search once installed; commands can be disabled. | [Manifest](https://developers.raycast.com/information/manifest), [Settings](https://manual.raycast.com/settings) | Plugin views are declared in the manifest and appear while the plugin is installed. |
| Root search ranking is synced across machines and cannot be turned off. | [Search Bar](https://manual.raycast.com/search-bar) | Usage history is user data: channel-neutral, one file. |

Not taken in 0.6: fallback commands for a query with no match ([Settings](https://manual.raycast.com/settings)), per-query learning (picks for a typed query ranking higher for that same query, [Search Bar](https://manual.raycast.com/search-bar)), aliases and hotkeys. Unverified: whether Raycast's root search shows a detail area; none of the pages read says so, and nothing here depends on it.

## Problems solved

| Part | What the person hits today |
|---|---|
| One component | Cmd+P, Cmd+O, Explorer search, Explorer actions and plugin lists each filter, select and close by different rules. |
| Field is the only writer | Typing `abc` in the Explorer can read `bca`: `handle_key` and the `TextEdit` both write `search_query`. |
| Files in Cmd+P | No file can be opened by name unless an Explorer is open. |
| Tab views | Notes, files and commands live behind separate keys and modals with separate ranking. |
| Agents | Agent rows appear in Cmd+P only behind `[agents] palette_agent_results` (default off), and only for running panes (`PaletteEntry::Agent` maps to `PaletteAction::Focus`). |
| Inline fields | `run_palette_selection` closes and fires once, so New file and Rename cannot run from the palette. |
| Docked Explorer | Cmd+E takes over the pane (`open_file_browser` passes `"overlay"`); hidden-files dies with the pane (`serialize_state`). |
| Preview | Two preview paths, both on the UI thread: `draw_notes_picker` reads the whole note through `notes_picker_preview_text`, and the Explorer decodes images and reads folders inside `draw_sidebar_preview` and `draw_quick_look_modal`. |

## Frames

```
Everything (Cmd+P, empty query)                  Files (Tab)
 [Everything] Agents Files Notes Commands         Everything Agents [Files] Notes Commands
 > |                                              > pick|
 Agents    reviewer   waiting on you              src/host/picker.rs     2m | //! Picker reducer
 Files     docs/RELEASE.md                        docs/palette-picker.md 1h | Stint: 1390 (design)
 Contexts  plexi-release          2 panes         walking...  (rows above came from the cache)
 Commands  Reload configuration     Cmd+Shift+,
 enter open   cmd+enter actions   tab views       enter open   cmd+enter actions   esc close
Agents (Tab)                                     Notes (Cmd+O)
 Everything [Agents] Files Notes Commands         Everything Agents Files [Notes] Commands
 > |                                              > stand|
 reviewer  waiting on you  2m | Ship 0.6 today?   Standup 11-12    | # Standup 11-12
 builder   working         5m | cargo test ...    Standing desk    | - palette design
 docs      idle            1h | Updated README    Stand-in  global | - cut 0.5 Groundwork
 planner   not running        |                   enter open   cmd+s open in split
 enter focus   cmd+enter actions   esc close
Commands (Cmd+Shift+P)                           Inline fields (Enter on "New file")
 Everything Agents Files Notes [Commands]         [New file]  in src/
 > new|                                           Name  > notes.md|
 New file         Explorer: src/         >        query "new" is kept behind this step
 New context               Cmd+Shift+N
 Rename           (needs a selected file)         enter create   esc back to results

Explorer (Cmd+E, right split, one folder)
 [Browse] Find Actions        src/host/
 > mod|
 Name            Size   Modified          rows painted by the Explorer table,
 model.rs        23k    29 Oct            selection and query owned by the picker
```

## State model

The picker is a pure reducer in a new `src/host/picker` module with no egui types. `HostModel` holds one `PickerState` per `PickerId` (`Palette`, `Pane(pane_id)`, `PluginNode(pane_id, node_id)`), reached through `HostAction::Picker(id, PickerCommand)` and answered with `HostEffect::Picker(id, PickerEffect)`. Today all of this is `PlexiApp.palette_*` and `PlexiApp.notes_picker_*` fields mutated inside draw functions, and `command_palette_handle_key` is an empty stub. `PickerState`: the view list and active view; one `query` shared by every view; `step` (`Results`, `Actions` with the row's action list, or `Fields` with the command, values, active field and error); ranked rows; `selected` as a row key, never an index; marked row keys (Explorer multi-select); a query generation; loading and preview status.

| Commands in | Effects out |
|---|---|
| `Open(view)`, `Close`, `NextView`, `PrevView`, `SetView` (chip click) | `Query { view, text, generation }` for the source runner |
| `QueryEdited(text)`, sent only by the text field | `LoadPreview(row key)` |
| `Move(delta)`, `Hover(row key)`, `Mark(row key)`, `ShowActions` | `Activate { row key, action id, marked, field values }` |
| `Accept(action id)`, `Back`, `FieldEdited`, `FieldNext`, `FieldPrev` | `Closed`, `Notice(text)` |
| `Rows { generation, rows, done }`, `Preview { row key, body }`, `SourceFailed` | |

Rules the reducer enforces:
- No key command edits `query`. Arrow, Enter, Tab and Esc never append or pop characters. The field is the pane's default text surface (`register_default_text_surface`), so it has focus whenever its host does and there is no handoff frame. In the `Actions` step the field filters the action list and the results query is kept behind it, as in the `Fields` step.
- Selection follows the row key across re-ranks. If the selected row disappears, nothing is selected until the person moves (today's `reconcile_palette_selection` rule).
- `Rows` from an older generation are dropped. Switching views keeps the query and re-emits `Query`. A `Preview` for a row that is no longer selected is dropped the same way.
- Actions never run in the reducer. `Activate` is resolved outside the model by the view's source.

## Views and sources

A view is data defined in code: `id`, chip label, source ids, ranking preset, whether it carries a preview, placeholder. The host views and their order are fixed: Everything, Agents, Files, Notes, Commands. A source is host code outside the model that answers `Query` with rows. Fast sources (commands, contexts, agents, plugin views, notes from the warm cache) answer in the same `App::logic` pass. The file source answers from a background worker drained in `App::logic`; cached rows show at once and `done` clears the walking line.

A row carries: `key` (`<source id>:<stable id>`; a source id never contains `:`, so the key splits on the first one), title, secondary text, chips, search text, score from `search::score_candidate`, modified time, group, an optional unavailable reason, a preview reference, and actions. An action is an id, a label and an optional chord; the first action is Enter. An action that needs input names its fields (label, kind `text`, `path` or `choice`, default, required); stint 1151 supplies these from the command registry. Unavailable rows stay listed, show the reason and refuse Enter. A source may return at most the row cap per query. A larger answer is refused with a typed error and a visible notice. The cap is `MAX_SEARCH_HITS` today; one constant for host and plugin sources.

| Ranking preset | Order | Used by |
|---|---|---|
| `relevance` | score across groups, then usage, then group order (stint 1115 contract) | Everything, Commands, Files |
| `title-first` | title contains, title fuzzy, body fuzzy (today's `notes_picker_filtered`) | Notes |
| `recent` | modified time, newest first, when the query is empty | Files with no query |
| `attention` | waiting on you first, then last interaction, newest first | Agents |
| `source-order` | the source's order, filtered only | plugin views that rank themselves |

| View | Shows | Preview |
|---|---|---|
| Everything | Notes, plugins, commands, user commands and contexts as today, plus files and agents. With an empty query, rows come in usage order, and files and agents appear only if they are in usage history. | none |
| Agents | See Agents view. | yes |
| Files | Files under the active context root, walked with `ScopedWalker` and its prune list. A file row's stable id is its canonical absolute path. | yes |
| Notes | Notes from every tier, as the notes picker does today. | yes |
| Commands | Registry commands and user commands. | none |

Everything has no preview because most of its rows (commands, contexts) have nothing to show, and a half that appears and disappears as the selection moves would shift the list.

## Usage ranking

Every `Activate` records the row key's use. Ranking uses it last, as the Prior art table says: after match score with a query, and as the whole order of Everything with an empty query, followed by source order.
- One file, `palette-usage.json`, in `shared_dir()` (`~/.plexi`, channel-neutral). Each entry is a row key, a pick count and the last pick time; entries unused for `USAGE_FORGET_AFTER` drop out. Only `src/host/picker` reads or writes it.
- It is user data, so per `src/host/AGENTS.md` it does not fork per channel. It is one global file rather than a `UserDataKind`: there is no per-context tier to resolve.
- Written off the UI thread, debounced, by atomic replace. Two channels running at once can each overwrite the other's latest picks; the file only orders rows, so the cost of a lost write is a slightly different order.
- "Reset ranking" removes the selected row's entry. A key whose row no longer exists simply never matches.

## Agents view

A first-class view: this is an agent operating system, and agents should be one Tab from Cmd+P rather than rows that are off by default. It is a picker projection of the roster the Agents pane already builds; it adds no second roster.

| Field | Comes from |
|---|---|
| Rows | `PlexiApp::agent_inputs`: every live agent pane in every context, with `context_name` as secondary text. Named definitions with no pane (`AgentRegistry`) appear as "not running" through the 1396 resolver. Closed runs in `fleet_history` stay in the Agents pane and are not rows. |
| Name | `AgentRow.role` (`resolve_agent_label`); search text also holds the pane name, harness and task title. |
| State | `DisplayState` with `AgentRow.needs_you`: waiting on you (`Blocked` with `needs_you`), blocked (`Blocked` without it, `BlockedOn::Agent` or `Message`), working, idle, done (`DoneUnseen`), failed (`Failed`, `Stalled`, `ExitedUnreported`), not running. `needs_you` already folds in open `NeedsYouRecord`s from `PermissionMonitor::open_needs_you`. |
| Latest message | `BlockedOn::label` when waiting, else the newest `AgentEvent` (the notification queue's projection), else `tail` (`capture_tail_preview`). |
| Preview | `text` kind: the pending question, the newest event body, or the tail lines. |
| Time | Shown as `format_age`. See Missing. |

Actions, in action-list order:
- **Focus** (Enter): focus the pane and `mark_agent_seen`. A not-running row runs the 1396 `agent open` resolver instead.
- **Answer** (first in the action list while waiting): an open needs-you item opens the Decision Overlay through `open_decision_overlay_for_ask`. The palette never resolves a permission itself; approvals stay on that trusted surface. `BlockedOn::Message` offers **Reply** with one inline text field, delivered by `reply_to_fleet_agent`.
- **New chat**: stint 1397.
- **Close pane**: the normal pane close path.

Missing, needed by the stint that lands this view:
- **Last interaction time.** `fleet_since` is the last *state* change (`Pane` sets it on every agent state transition), not the last time the person typed into, focused or answered the agent. Add one per-pane `last_interaction_at` set on those three events and persisted with the pane like `fleet_since`. Until it exists, `attention` would sort by state changes, which a busy agent produces constantly.
- **Stop.** No typed host action stops an agent's turn while keeping its session. Close pane ends it. A harness-aware stop needs its own stint.
- **Snapshot cost.** `agent_inputs` loads `PermissionMonitor` from disk on every call. The source must answer from a snapshot refreshed when agent state or the needs-you set changes, not rebuild it on every keystroke.

Chord: none new in 0.6. Agents is one Tab from Cmd+P. Cmd+Alt+A keeps opening the Agents pane; see Decisions.

## Preview

The source decides what a row's preview is. When it builds a row it sets the preview reference: a kind and a target only the source understands (a path, a note path, a pane id, a plugin handle). The `Preview` painter draws by kind and never guesses from a file name. For files, one classifier shared by the file source and the Explorer picks the kind; it replaces the separate chains in `draw_sidebar_preview` and `draw_quick_look_modal`, `is_text_preview_candidate`, and the two image-extension lists in `src/file_browser/mod.rs`. In a view with a preview, the preview always shows; a row with no supported kind shows the empty state.

| Kind | Shown | Limits |
|---|---|---|
| `text` | wrapped monospace, scrollable. Markdown shows as its source text, as the notes picker does today; notes show `notes::note_body`. | `TEXT_PREVIEW_MAX_BYTES`, then a "truncated" line (`read_text_preview`; notes use it too instead of reading the whole file). A NUL byte makes it unsupported: "Binary file". |
| `image` | fitted to the preview half, aspect kept | decoded and downscaled off the UI thread to the longest edge `ensure_image_preview` uses today, made a named constant. A file over `PREVIEW_IMAGE_MAX_BYTES` shows "Too large to preview". |
| `folder` | file and folder counts and total size, the summary Explorer Quick Look shows today (`DirStats`, `draw_quick_look_folder`) | stops at `DIR_PREVIEW_CAP` entries. Kept only so Explorer Quick Look does not lose what it shows today; it is not an entry listing. |

Anything else shows one dim line: the source's reason, else "No preview".

Loading:
- `LoadPreview` goes to the row's source, which reads on a preview worker, never the UI thread. Results drain in `App::logic` as `Preview { row key, body }`; an image texture is uploaded there too, since `App::logic` has the egui context.
- While a preview loads, the half shows the row title and a dim "Loading". A read that has not answered within `PREVIEW_TIMEOUT` shows "Preview timed out". A failed read shows "Could not read: <error>" and logs the path and error at `warn`. None of these blocks the results or the next selection.
- Below `MODAL_WIDTH_MD` the preview half is dropped, as `draw_notes_picker` does today.

Stint 1116 (0.8 Writing Desk, images attached to notes) lands in this `text` kind for notes: inline images through the editor's `preview::image_spans` and `ImageCache`, loaded by the same preview worker. It must not add a second notes preview path; `draw_notes_picker` is deleted by 1391.

## Plugin palette views

The API belongs to stint 1395 (0.7 Workbench). What 0.6 fixes so that 1395 does not have to undo it:
- **Declaration.** A plugin declares a palette view in its manifest as its own contribution kind, with the fields of a host view record (id, label, preset, preview); the host builds the view with a plugin source. `manifest_contract` rejects unknown contribution kinds as "not yet supported" today; 1395 adds this kind and its validation.
- **Presence.** The view follows the host views, in plugin id order, while the plugin is installed (and enabled, once plugins can be disabled). It goes away with the plugin. There is no palette setting. Rows from it recorded in usage history simply stop matching.
- **Id.** Plugin-namespaced like command ids (`<plugin id>.<suffix>`), so it never collides with a host view id. The same id is the source id in row keys.
- **Capability.** Declaring a view needs none; a listing grants nothing (`docs/prm/permissions-broker.md`, command registry). Activating a row runs the plugin's own handler, or a registry command through the broker under the plugin's grants; a plugin row cannot carry a host action.
- **Host guarantees.** The row cap applies and an oversized answer is refused with a typed error. A query unanswered within `PLUGIN_SOURCE_TIMEOUT` shows "<plugin> did not answer" in that view only. A crash or error is `SourceFailed`, shown in that view only. Other views never wait on a plugin: each view queries only its own sources, and plugin views do not feed Everything in 0.6. Plugin previews are `text` only, under the same byte cap, unless 1395 adds kinds.

## Agent and CLI access

Out of scope for 0.6 Palette. Agents already read the agent roster through `plexi agent fleet --json` and run commands through the 1151 registry, which is the record palette, keybindings, CLI and agent calls all invoke. The intent for later is that a CLI query reaches the same sources and returns the same row keys and action ids, so an agent picks exactly what a person would; no stint owns that yet.

## Keyboard map

Chords checked in `KeyBindings::default`. Cmd+Shift+P has no binding today; it becomes `open_command_palette_commands`.

| Key | Results step | Actions and Fields steps |
|---|---|---|
| Cmd+P, Cmd+Shift+P, Cmd+O | open the palette on Everything, Commands, Notes; the same chord closes it | same |
| Cmd+E | open or close the Explorer right split (layout `split_right`) | same |
| Tab, Shift+Tab | next view, previous view | Fields: next field, previous field |
| Up, Down, Cmd+K, Cmd+J | move selection | move in the action list; unchanged in a choice field |
| Enter | first action; if it needs input, go to the fields step | run the action; Fields: next field, run on the last |
| Cmd+Enter | list the selected row's actions | none |
| Esc | modal: close. Docked: clear the query, then pass to the host | back to results, query and selection kept |
| Action chords | declared by the view. Notes keeps Cmd+S, Cmd+Backspace, Cmd+Z, copy and cut | none |

- `/` and `>` are ordinary characters. No prefix switches a view. Shift+Enter is reserved for "open in the other placement" (stint 1245, 0.7 Workbench).
- Cmd+Enter is `zoom_in` among host bindings. The modal wins because `poll_actions` skips `Normal` bindings while an overlay is open; the docked Explorer claims it with the chord claim below.
- A picker has no bare-key shortcuts. Space, Left, Right, h, j, k and l go to the field. Explorer Quick Look is Cmd+Y; parent folder and the rest become registry commands with chords, picked in the stint 1394 note. Parent folder proposal: Cmd+Up, unbound today.
- Cmd+J and Cmd+K are host pane navigation. A docked picker needs the typed chord claim from stint 1110: a focused view declares the chords it owns, `poll_actions` skips those bindings, and `dispatch_plugin_key_events` forwards them even while the field has focus. `keyboard_capture` stays all-or-nothing and is not used for this.

## Three hosts, one component

`src/ui/picker.rs` has painters that read a `PickerState` and return `PickerCommand`s: `Field` (chips, `TextField`, actions and fields steps), `Results` (`ListRow`s with group headers, scroll to selection, hover and click) and `Preview` (draws the kinds above). Keys are translated in the focus owner's `handle_key`, never in a draw function. The seam for 1395: rows, row keys, action ids, preset names, preview kind names and the row cap contain no host paths, no egui types and no host enums, so they can cross WIT unchanged.

| Host | Shell | Parts | Notes |
|---|---|---|---|
| Palette | `ModalShell::centered`, `FocusKind::CommandPalette` | Field, Results, Preview in views that carry it | Opening keeps the invoking pane and selection as the command target. |
| Explorer | `FileBrowserView` pane, `PickerId::Pane` | Field on top; the Explorer's own table paints the rows | Views Browse (this folder), Find (below this folder), Actions (commands targeting the selection). Preferences move from pane state to an `[explorer]` table in `config.toml`, written through `config::edit` like `ui_scale`. Config is per channel, so Alpha and Beta remember separately. |
| Plugin Picker | a `Picker` node in a plugin pane (stint 1395) | Field, Results, Preview | The plugin sends rows and a preset name and receives `query` and `picked` events. A plugin palette view is the same view record with a plugin source. |

## Deleted

- Stint 1391: `PaletteEntry`, `PaletteAction`, `action_for_palette_entry`, `sort_palette_entries`, `reconcile_palette_selection`, the key block in `draw_command_palette`, `command_palette_handle_key`, `shortcut_label`, the `palette_query`, `palette_selected`, `palette_selected_key` and `palette_scroll_reset` fields; `draw_notes_picker`, `notes_picker_handle_key`, `notes_picker_filtered`, `notes_picker_apply_query`, `notes_picker_preview_text`, `NotesPickerOwner`, and the `notes_picker_query`, `notes_picker_selected`, `notes_picker_last_drawn_selected`, `notes_picker_shown_paths` and `notes_picker_preview` fields. Note delete, undo, copy and cut stay as actions.
- The Agents view stint: the `[agents] palette_agent_results` key, from `src/config/mod.rs`, `scripts/default-config.toml` and `docs/CONFIG.md`.
- Stints 1151 and 1393: `PaletteCommand`, `PaletteCommandEntry`, `PALETTE_COMMANDS`, `run_palette_selection`, `run_palette_command`.
- Stint 1394: all of `src/file_browser/intent.rs`; in the Explorer `Mode`, the `AppendText`, `Backspace`, `EnterSearch`, `EnterPaletteSlash`, `EnterPaletteCommand` and `TabComplete` arms of `classify_key` and `handle_key`, `draw_search_bar`, `draw_action_palette`, `palette_preview_chip`, `open_palette`, `exit_palette`, `rank_palette_candidates`, `tab_complete_palette`, `submit_palette`, `parse_intent`, `intent_context`, `execute_intent`, `enter_search`, `exit_search`, `draw_rename_modal`, `open_rename_modal`, `confirm_rename_modal`, `cancel_rename_modal`, `toggle_legacy_sort`, `PALETTE_MAX_VISIBLE_CANDIDATES`, `ensure_image_preview`, `ensure_dir_preview`, `ensure_text_preview`, and the `in_search`, `in_palette`, `search_query`, `palette_*`, `search_focus_pending`, `preview_*`, `dir_preview_*` and `text_preview*` fields.

## Build order

| Stint | Lands | Tests that must pass |
|---|---|---|
| 1396 named agent launch (first; edits `command_palette.rs`) | Agent rows keyed by agent name; one `agent open` resolver shared by palette and CLI | `named_agent_focuses_when_running`, `named_agent_resumes_last_chat_when_stopped`, `named_agent_starts_new_when_never_run`, `cli_agent_open_matches_palette`, `ui_tests::palette_lists_defined_agents` |
| 1391 picker core | Reducer, painters, action list, views Everything, Commands, Notes, Cmd+Shift+P; preview worker with the `text` kind; usage ranking | `tab_cycles_views_and_shift_tab_goes_back`, `query_survives_view_switch`, `no_key_command_edits_the_query`, `selection_follows_row_key_and_clears_on_removal`, `stale_generation_rows_are_dropped`, `mouse_and_enter_resolve_the_same_action`, `cmd_enter_lists_actions_and_keeps_the_query`, `cmd_shift_p_opens_on_commands`, `preview_never_loads_on_the_ui_thread`, `preview_for_a_moved_selection_is_dropped`, `empty_query_orders_everything_by_usage`, `usage_breaks_ties_after_score`, `reset_ranking_forgets_the_row`, `usage_file_is_channel_neutral`, `ui_tests::palette_typing_abc_reads_abc`, `ui_tests::palette_everything_view`, `ui_tests::palette_commands_view`, `ui_tests::palette_notes_view_with_preview` |
| Agents view (new stint; after 1391 and 1396) | Agents view, `attention` preset, `last_interaction_at`, Answer, Reply and Close actions, snapshot source, `palette_agent_results` deleted | `agents_waiting_on_you_sort_first`, `agents_sort_by_last_interaction_not_state_change`, `answer_opens_the_decision_overlay_and_never_resolves_in_the_palette`, `reply_delivers_through_reply_to_fleet_agent`, `agents_source_does_not_read_disk_per_query`, `ui_tests::palette_agents_view_with_preview` |
| 1392 files | Shared file index, Files view, files in Everything, `image` and `folder` kinds, the shared file classifier | `file_walk_never_runs_on_the_ui_thread`, `palette_opens_a_file_with_no_explorer_open`, `cached_rows_show_before_the_walk_finishes`, `oversized_source_answer_is_refused`, `oversized_image_shows_no_preview_with_reason`, `ui_tests::palette_files_view_with_preview` |
| 1393 inline fields (after 1151) | Fields step fed by registry argument schemas | `enter_on_command_with_input_moves_to_fields`, `esc_in_fields_returns_with_query_and_selection`, `required_field_blocks_run_and_says_why`, `target_captured_at_enter_is_rechecked_at_run`, `new_file_and_rename_run_from_the_palette`, `ui_tests::palette_inline_fields` |
| 1394 Explorer | Docked picker, right split, config preferences, chord claim, one open resolver, Quick Look on Cmd+Y over the shared preview loader, parser deleted | `cmd_e_opens_a_right_split`, `explorer_typing_abc_reads_abc`, `hidden_files_choice_survives_restart_and_folder_change`, `explorer_and_notes_open_through_one_resolver`, `focused_picker_owns_cmd_j_k_and_gives_them_back`, `directory_read_never_runs_on_the_ui_thread`, `quick_look_uses_the_shared_preview_loader`, `ui_tests::explorer_browse_view`, `ui_tests::explorer_find_view`, `ui_tests::explorer_actions_view` |
| 1397 agent memory and New chat | "New chat" as an Agents row action and a command | `new_chat_archives_the_current_chat`, `archived_chat_reopens_from_history`, `new_chat_reads_the_same_memory_folder` |
| 1398 installs off (separate lane) | Host switch; the palette may open the setting but never flip it | `non_bundled_install_is_refused_with_a_typed_error`, `cli_cannot_bypass_the_install_switch`, `palette_cannot_flip_the_install_switch` |
| 1395 public Picker (0.7 Workbench) | Palette view contribution kind, plugin sources, `PLUGIN_SOURCE_TIMEOUT` | `plugin_view_follows_host_views_while_installed`, `slow_plugin_source_never_blocks_other_views`, `failed_plugin_source_shows_only_in_its_view` |

1396 stays first and separate: its resolver and CLI parity stand alone and land before the reducer. 1397 stays separate: its work is chat memory, and New chat is one action the Agents view lists.

## Technical choices

| Choice | Options | Pick |
|---|---|---|
| Where picker state lives | A. Stay on `PlexiApp`: no move, but logic stays inside draw code and needs a UI harness to test. B. Each host owns its own `PickerState`: simple ownership, three places to service. C. `HostModel` keyed by `PickerId`: one reducer, plain Rust tests, state dies with its pane. | C |
| How sources reach the model | A. A trait the reducer calls: direct, but disk and plugin calls enter the model. B. Effects out, rows in: the model stays pure and slow sources are naturally off-thread. C. Sources push whole lists each frame (today): re-ranks every frame. | B |
| Ranking | A. A closure per source: flexible, cannot cross to plugins. B. One ranking everywhere: notes lose title-first order. C. Named presets over one scorer, usage last. | C |
| File index | A. A `SearchSession` per picker (Explorer today): two walks of the same root. B. One shared index per root, kept by the host and refreshed in the background on open. C. An index on disk: fast cold start, new format to own. | B |
| Explorer rows | A. The component paints a table too: larger kit surface now. B. The Explorer keeps `draw_details_table` and reads rows and selection from the picker. C. Explorer rows become `ListRow`s: loses columns. | B until 1.0 |
| Action payload | A. Rows carry host enums (`PaletteAction`): cannot cross to plugins. B. Rows carry a row key and action id, resolved by the source; the id is a registry command id once 1151 lands. | B |
| Preview kind | A. The painter guesses from the file name: works only for files, cannot cross to plugins. B. The source sets the kind on the row: one rule for host and plugin rows. | B |

## Decisions for Ian

| # | Decision | Options | My pick |
|---|---|---|---|
| 1 | What replaces pinned lists in 0.6. | A. Usage ranking only: empty Everything ordered by what you pick most and most recently, one channel-neutral file. B. A plus a "Favorite" action that floats rows to the top of empty Everything (Raycast's Cmd+F), stored as a flag in the same file. C. Neither. | A. It needs no new verb; a Favorite flag can be added to the same file later if usage ordering is not enough. |
| 2 | Which chord reaches the Agents view. | A. None: one Tab from Cmd+P; Cmd+Alt+A keeps opening the Agents pane. B. Cmd+Alt+A opens the palette on Agents, and the Agents pane opens from a command. C. A new chord. | A for 0.6; revisit after using it. |

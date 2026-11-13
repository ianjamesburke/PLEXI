Stint: 1390 (design), built by 1391, 1392, 1393, 1394
Status: active

# One picker: the palette, the Explorer box and the plugin Picker

Release 0.6 Palette. Rulings: ledger rows 42, 68, 69, 70, 75. Checked against `alpha` on 2026-11-13.

## Problems solved

| Part | What the person hits today |
|---|---|
| One component | Cmd+P, Cmd+O, Explorer search, Explorer actions and plugin lists each filter, select and close by different rules. |
| Field is the only writer | Typing `abc` in the Explorer can read `bca`: `handle_key` and the `TextEdit` both write `search_query`. |
| Files in Cmd+P | No file can be opened by name unless an Explorer is open. |
| Tab views | Notes, files and commands live behind separate keys and modals with separate ranking. |
| Inline fields | `run_palette_selection` closes and fires once, so New file and Rename cannot run from the palette. |
| Docked Explorer | Cmd+E takes over the pane (`open_file_browser` passes `"overlay"`); hidden-files dies with the pane (`serialize_state`). |
| Named agent rows | Agent rows are running panes only (`PaletteEntry::Agent` maps to `PaletteAction::Focus`). |

## Frames

```
Everything (Cmd+P)                               Files (Tab)
 [Everything] Files Commands Notes Pinned         Everything [Files] Commands Notes Pinned
 > rel|                                           > pick|
 Contexts  plexi-release          2 panes         src/host/picker.rs              2m ago
 Agents    reviewer      resume last chat         docs/palette-picker.md          1h ago
 Files     docs/RELEASE.md                        walking...  (rows above came from the cache)
 Commands  Reload configuration     Cmd+Shift+,
 enter open   tab views   esc close               enter open   tab views   esc close
Commands (Cmd+Shift+P)                           Notes (Cmd+O)
 Everything Files [Commands] Notes Pinned         Everything Files Commands [Notes] Pinned
 > new|                                           > stand|
 New file         Explorer: src/         >        Standup 11-12    | # Standup 11-12
 New context               Cmd+Shift+N            Standing desk    | - palette design
 Rename           (needs a selected file)         Stand-in  global | - cut 0.5 Groundwork
 enter run   > takes input                        enter open   cmd+s open in split
Pinned                                           Inline fields (Enter on "New file")
 Everything Files Commands Notes [Pinned]         [New file]  in src/
 > |                                              Name  > notes.md|
 Release      docs/RELEASE.md                     query "new" is kept behind this step
 Release      Reload configuration
 Daily        reviewer                            enter create   esc back to results

Explorer (Cmd+E, right split, one folder)
 [Browse] Find Actions        src/host/
 > mod|
 Name            Size   Modified          rows painted by the Explorer table,
 model.rs        23k    29 Oct            selection and query owned by the picker
```

## State model

The picker is a pure reducer in a new `src/host/picker` module with no egui types. `HostModel` holds one `PickerState` per `PickerId` (`Palette`, `Pane(pane_id)`, `PluginNode(pane_id, node_id)`), reached through `HostAction::Picker(id, PickerCommand)` and answered with `HostEffect::Picker(id, PickerEffect)`. Today all of this is `PlexiApp.palette_*` and `PlexiApp.notes_picker_*` fields mutated inside draw functions, and `command_palette_handle_key` is an empty stub. `PickerState`: the view list and active view; one `query` shared by every view; `step` (`Results`, or `Fields` with the command, values, active field and error); ranked rows; `selected` as a row key, never an index; marked row keys (Explorer multi-select); a query generation; loading and preview status.

| Commands in | Effects out |
|---|---|
| `Open(view)`, `Close`, `NextView`, `PrevView`, `SetView` (chip click) | `Query { view, text, generation }` for the source runner |
| `QueryEdited(text)`, sent only by the text field | `LoadPreview(row key)` |
| `Move(delta)`, `Hover(row key)`, `Mark(row key)` | `Activate { row key, action id, marked, field values }` |
| `Accept(action id)`, `Back`, `FieldEdited`, `FieldNext`, `FieldPrev` | `Closed`, `Notice(text)` |
| `Rows { generation, rows, done }`, `Preview { row key, body }`, `SourceFailed` | |

Rules the reducer enforces:
- No key command edits `query`. Arrow, Enter, Tab and Esc never append or pop characters. The field is the pane's default text surface (`register_default_text_surface`), so it has focus whenever its host does and there is no handoff frame.
- Selection follows the row key across re-ranks. If the selected row disappears, nothing is selected until the person moves (today's `reconcile_palette_selection` rule).
- `Rows` from an older generation are dropped. Switching views keeps the query and re-emits `Query`.
- Actions never run in the reducer. `Activate` is resolved outside the model by the view's source.

## Views and sources

A view is data: `id`, chip label, source ids, ranking preset, `preview` on or off, placeholder. A source is host code outside the model that answers `Query` with rows. Fast sources (commands, contexts, agents, plugin views, notes from the warm cache) answer in the same `App::logic` pass. The file source answers from a background worker drained in `App::logic`; cached rows show at once and `done` clears the walking line.

A row carries: `key` (`<source id>:<stable id>`), title, secondary text, chips, search text, score from `search::score_candidate`, modified time, group, an optional unavailable reason, a preview reference, and actions. An action is an id, a label and an optional chord; the first action is Enter. An action that needs input names its fields (label, kind `text`, `path` or `choice`, default, required); stint 1151 supplies these from the command registry. Unavailable rows stay listed, show the reason and refuse Enter. A source may return at most the row cap per query. A larger answer is refused with a typed error and a visible notice. The cap is `MAX_SEARCH_HITS` today; one constant for host and plugin sources.

| Ranking preset | Order | Used by |
|---|---|---|
| `relevance` | score across groups, group order breaks ties (stint 1115 contract) | Everything, Commands, Files |
| `title-first` | title contains, title fuzzy, body fuzzy (today's `notes_picker_filtered`) | Notes |
| `recent` | modified time, newest first, when the query is empty | Files with no query |
| `source-order` | the source's order, filtered only | Pinned, plugin views that rank themselves |

Everything shows contexts, agents, notes, plugins, commands and user commands as today, plus files once the query is not empty. Files walk the active context root with `ScopedWalker` and its prune list. Pinned shows every pinned list as a group. A row is pinned by the registry command "Pin to list", which takes the list name as an inline field.

## Keyboard map

Chords checked in `KeyBindings::default`. Cmd+Shift+P has no binding today; it becomes `open_command_palette_commands`.

| Key | Results step | Fields step |
|---|---|---|
| Cmd+P, Cmd+Shift+P, Cmd+O | open the palette on Everything, Commands, Notes; the same chord closes it | same |
| Cmd+E | open or close the Explorer right split (layout `split_right`) | same |
| Tab, Shift+Tab | next view, previous view | next field, previous field |
| Up, Down, Cmd+K, Cmd+J | move selection | unchanged in a choice field |
| Enter | first action; if it needs input, go to the fields step | next field; run on the last |
| Esc | modal: close. Docked: clear the query, then pass to the host | back to results, query and selection kept |
| Action chords | declared by the view. Notes keeps Cmd+S, Cmd+Backspace, Cmd+Z, copy and cut | none |

- `/` and `>` are ordinary characters. No prefix switches a view. Shift+Enter is reserved for "open in the other placement" (stint 1245, 0.7 Workbench).
- A picker has no bare-key shortcuts. Space, Left, Right, h, j, k and l go to the field. The Explorer's Quick Look, parent folder and the rest become registry commands with chords, picked in the stint 1394 note. Parent folder proposal: Cmd+Up, unbound today.
- Cmd+J and Cmd+K are host pane navigation. The modal already wins because `poll_actions` skips `Normal` bindings while an overlay is open. A docked picker needs the typed chord claim from stint 1110: a focused view declares the chords it owns, `poll_actions` skips those bindings, and `dispatch_plugin_key_events` forwards them even while the field has focus. `keyboard_capture` stays all-or-nothing and is not used for this.

## Three hosts, one component

`src/ui/picker.rs` has painters that read a `PickerState` and return `PickerCommand`s: `Field` (chips, `TextField`, fields step), `Results` (`ListRow`s with group headers, scroll to selection, hover and click) and `Preview`. Keys are translated in the focus owner's `handle_key`, never in a draw function. The seam for 1395: rows, row keys, action ids, preset names and the row cap contain no host paths, no egui types and no host enums, so they can cross WIT unchanged.

| Host | Shell | Parts | Notes |
|---|---|---|---|
| Palette | `ModalShell::centered`, `FocusKind::CommandPalette` | Field, Results, Preview when the view asks | Opening keeps the invoking pane and selection as the command target. |
| Explorer | `FileBrowserView` pane, `PickerId::Pane` | Field on top; the Explorer's own table paints the rows | Views Browse (this folder), Find (below this folder), Actions (commands targeting the selection). Preferences move from pane state to an `[explorer]` table in `config.toml`, written through `config::edit` like `ui_scale`. Config is per channel, so Alpha and Beta remember separately. |
| Plugin Picker | a `Picker` node in a plugin pane (stint 1395) | Field, Results, Preview | The plugin sends rows and a preset name and receives `query` and `picked` events. A plugin palette view is the same view record with a plugin source. |

## Deleted

- Stint 1391: `PaletteEntry`, `PaletteAction`, `action_for_palette_entry`, `sort_palette_entries`, `reconcile_palette_selection`, the key block in `draw_command_palette`, `command_palette_handle_key`, `shortcut_label`, the `palette_query`, `palette_selected`, `palette_selected_key` and `palette_scroll_reset` fields; `draw_notes_picker`, `notes_picker_handle_key`, `notes_picker_filtered`, `notes_picker_apply_query`, `NotesPickerOwner`, and the `notes_picker_query`, `notes_picker_selected`, `notes_picker_last_drawn_selected` and `notes_picker_shown_paths` fields. Note delete, undo, copy and cut stay as actions.
- Stints 1151 and 1393: `PaletteCommand`, `PaletteCommandEntry`, `PALETTE_COMMANDS`, `run_palette_selection`, `run_palette_command`.
- Stint 1394: all of `src/file_browser/intent.rs`; in the Explorer `Mode`, the `AppendText`, `Backspace`, `EnterSearch`, `EnterPaletteSlash`, `EnterPaletteCommand` and `TabComplete` arms of `classify_key` and `handle_key`, `draw_search_bar`, `draw_action_palette`, `palette_preview_chip`, `open_palette`, `exit_palette`, `rank_palette_candidates`, `tab_complete_palette`, `submit_palette`, `parse_intent`, `intent_context`, `execute_intent`, `enter_search`, `exit_search`, `draw_rename_modal`, `open_rename_modal`, `confirm_rename_modal`, `cancel_rename_modal`, `toggle_legacy_sort`, `PALETTE_MAX_VISIBLE_CANDIDATES`, and the `in_search`, `in_palette`, `search_query`, `palette_*` and `search_focus_pending` fields.

## Build order

| Stint | Lands | Tests that must pass |
|---|---|---|
| 1396 named agent launch (first; edits `command_palette.rs`) | Agent rows keyed by agent name; one `agent open` resolver shared by palette and CLI | `named_agent_focuses_when_running`, `named_agent_resumes_last_chat_when_stopped`, `named_agent_starts_new_when_never_run`, `cli_agent_open_matches_palette`, `ui_tests::palette_lists_defined_agents` |
| 1391 picker core | Reducer, painters, views Everything, Commands, Notes, Cmd+Shift+P | `tab_cycles_views_and_shift_tab_goes_back`, `query_survives_view_switch`, `no_key_command_edits_the_query`, `selection_follows_row_key_and_clears_on_removal`, `stale_generation_rows_are_dropped`, `mouse_and_enter_resolve_the_same_action`, `cmd_shift_p_opens_on_commands`, `ui_tests::palette_typing_abc_reads_abc`, `ui_tests::palette_everything_view`, `ui_tests::palette_commands_view`, `ui_tests::palette_notes_view_with_preview` |
| 1392 files and pinned | Shared file index, Files view, files in Everything, Pinned view | `file_walk_never_runs_on_the_ui_thread`, `palette_opens_a_file_with_no_explorer_open`, `cached_rows_show_before_the_walk_finishes`, `pinned_list_survives_restart`, `oversized_source_answer_is_refused`, `ui_tests::palette_files_view`, `ui_tests::palette_pinned_view` |
| 1393 inline fields (after 1151) | Fields step fed by registry argument schemas | `enter_on_command_with_input_moves_to_fields`, `esc_in_fields_returns_with_query_and_selection`, `required_field_blocks_run_and_says_why`, `target_captured_at_enter_is_rechecked_at_run`, `new_file_and_rename_run_from_the_palette`, `ui_tests::palette_inline_fields` |
| 1394 Explorer | Docked picker, right split, config preferences, chord claim, one open resolver, parser deleted | `cmd_e_opens_a_right_split`, `explorer_typing_abc_reads_abc`, `hidden_files_choice_survives_restart_and_folder_change`, `explorer_and_notes_open_through_one_resolver`, `focused_picker_owns_cmd_j_k_and_gives_them_back`, `directory_read_never_runs_on_the_ui_thread`, `ui_tests::explorer_browse_view`, `ui_tests::explorer_find_view`, `ui_tests::explorer_actions_view` |
| 1397 agent memory and New chat | "New chat" as an agent row action and a command | `new_chat_archives_the_current_chat`, `archived_chat_reopens_from_history`, `new_chat_reads_the_same_memory_folder` |
| 1398 installs off (separate lane) | Host switch; the palette may open the setting but never flip it | `non_bundled_install_is_refused_with_a_typed_error`, `cli_cannot_bypass_the_install_switch`, `palette_cannot_flip_the_install_switch` |

## Technical choices

| Choice | Options | Pick |
|---|---|---|
| Where picker state lives | A. Stay on `PlexiApp`: no move, but logic stays inside draw code and needs a UI harness to test. B. Each host owns its own `PickerState`: simple ownership, three places to service. C. `HostModel` keyed by `PickerId`: one reducer, plain Rust tests, state dies with its pane. | C |
| How sources reach the model | A. A trait the reducer calls: direct, but disk and plugin calls enter the model. B. Effects out, rows in: the model stays pure and slow sources are naturally off-thread. C. Sources push whole lists each frame (today): re-ranks every frame. | B |
| Ranking | A. A closure per source: flexible, cannot cross to plugins. B. One ranking everywhere: notes lose title-first order. C. Named presets over one scorer. | C |
| File index | A. A `SearchSession` per picker (Explorer today): two walks of the same root. B. One shared index per root, kept by the host and refreshed in the background on open. C. An index on disk: fast cold start, new format to own. | B |
| Explorer rows | A. The component paints a table too: larger kit surface now. B. The Explorer keeps `draw_details_table` and reads rows and selection from the picker. C. Explorer rows become `ListRow`s: loses columns. | B until 1.0 |
| Action payload | A. Rows carry host enums (`PaletteAction`): cannot cross to plugins. B. Rows carry a row key and action id, resolved by the source; the id is a registry command id once 1151 lands. | B |

## Decisions for Ian

| # | Decision | Options | My pick |
|---|---|---|---|
| 1 | Where pinned lists are stored. This becomes a user-data address. | New `UserDataKind` `pins`, one TOML file per list, global and context tiers both listed; or a table in `config.toml`; or plugin state | New `pins` kind |
| 2 | Row key format. Pinned lists write it to disk and plugins will send it. | `<source id>:<stable id>` strings; or typed variants per source | Strings |
| 3 | Ranking preset names. They become plugin API words in 0.7 Workbench. | `relevance`, `title-first`, `recent`, `source-order`; or a larger tunable set | These four |
| 4 | Explorer loses bare-key shortcuts (Space for Quick Look, h and l, Left and Right) because typing always filters. | Accept, with chords for each; or keep a browse mode where letters are shortcuts | Accept |

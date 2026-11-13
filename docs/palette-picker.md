Stint: 1390 (design), built by 1391, 1392, 1393, 1394
Status: active

# One picker: the palette, the Explorer box and the plugin Picker

Release 0.6 Palette. Rulings: ledger rows 42, 68, 69, 70, 75. Checked against `alpha` on 2026-11-13.

## How it fits together

One field takes the person's text. Tab and Shift+Tab move between views; a view is a named set of sources with a ranking preset. Sources answer the query with rows. Enter or an action chord runs the selected row's action, and an action that needs input opens inline fields under the same field without losing the query. When the view's preview is open, the selected row's preview shows beside the results. `[palette]` in `config.toml` sets which views Tab reaches and in what order, and plugin views join that same list. One reducer and one set of painters serve three hosts: the palette modal (Cmd+P, Cmd+Shift+P, Cmd+O), the Explorer docked in a right split (Cmd+E), and a plugin's Picker node.

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
| Preview | Two preview paths, both on the UI thread: `draw_notes_picker` reads the whole note through `notes_picker_preview_text`, and the Explorer decodes images and reads folders inside `draw_sidebar_preview` and `draw_quick_look_modal`. |

## Frames

```
Everything (Cmd+P)                               Files (Tab)
 [Everything] Files Commands Notes Pinned         Everything [Files] Commands Notes Pinned
 > rel|                                           > pick|
 Contexts  plexi-release          2 panes         src/host/picker.rs     2m | //! Picker reducer
 Agents    reviewer      resume last chat         docs/palette-picker.md 1h | Stint: 1390 (design)
 Files     docs/RELEASE.md                        walking...  (rows above came from the cache)
 Commands  Reload configuration     Cmd+Shift+,
 enter open   tab views   esc close               enter open   cmd+y preview   esc close
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
| `Move(delta)`, `Hover(row key)`, `Mark(row key)`, `TogglePreview` | `Activate { row key, action id, marked, field values }` |
| `Accept(action id)`, `Back`, `FieldEdited`, `FieldNext`, `FieldPrev` | `Closed`, `Notice(text)`, `PreviewToggled { view, open }` |
| `Rows { generation, rows, done }`, `Preview { row key, body }`, `SourceFailed` | |

Rules the reducer enforces:
- No key command edits `query`. Arrow, Enter, Tab and Esc never append or pop characters. The field is the pane's default text surface (`register_default_text_surface`), so it has focus whenever its host does and there is no handoff frame.
- Selection follows the row key across re-ranks. If the selected row disappears, nothing is selected until the person moves (today's `reconcile_palette_selection` rule).
- `Rows` from an older generation are dropped. Switching views keeps the query and re-emits `Query`. A `Preview` for a row that is no longer selected is dropped the same way.
- Actions never run in the reducer. `Activate` is resolved outside the model by the view's source.

## Views and sources

A view is data: `id`, chip label, source ids, ranking preset, default preview open or closed, placeholder. A source is host code outside the model that answers `Query` with rows. Fast sources (commands, contexts, agents, plugin views, notes from the warm cache) answer in the same `App::logic` pass. The file source answers from a background worker drained in `App::logic`; cached rows show at once and `done` clears the walking line.

A row carries: `key` (`<source id>:<stable id>`; a source id never contains `:`, so the key splits on the first one), title, secondary text, chips, search text, score from `search::score_candidate`, modified time, group, an optional unavailable reason, a preview reference, and actions. An action is an id, a label and an optional chord; the first action is Enter. An action that needs input names its fields (label, kind `text`, `path` or `choice`, default, required); stint 1151 supplies these from the command registry. Unavailable rows stay listed, show the reason and refuse Enter. A source may return at most the row cap per query. A larger answer is refused with a typed error and a visible notice. The cap is `MAX_SEARCH_HITS` today; one constant for host and plugin sources.

| Ranking preset | Order | Used by |
|---|---|---|
| `relevance` | score across groups, group order breaks ties (stint 1115 contract) | Everything, Commands, Files |
| `title-first` | title contains, title fuzzy, body fuzzy (today's `notes_picker_filtered`) | Notes |
| `recent` | modified time, newest first, when the query is empty | Files with no query |
| `source-order` | the source's order, filtered only | Pinned, plugin views that rank themselves |

Host view ids are `everything`, `files`, `commands`, `notes` and `pinned`. Everything shows contexts, agents, notes, plugins, commands and user commands as today, plus files once the query is not empty. Files walk the active context root with `ScopedWalker` and its prune list; a file row's stable id is its canonical absolute path, so a pinned file opens from any context. Pinned shows every pinned list from `config.toml` as a group (see Configuration).

## Preview

The source decides what a row's preview is. When it builds a row it sets the preview reference: a kind and a target the source alone understands (a path, a note path, a plugin handle). The `Preview` painter draws by kind and never guesses from a file name. For files, one classifier shared by the file source and the Explorer picks the kind; it replaces the separate chains in `draw_sidebar_preview` and `draw_quick_look_modal`, `is_text_preview_candidate`, and the two image-extension lists in `src/file_browser/mod.rs`.

| Kind | Shown | Set by | Limits |
|---|---|---|---|
| `text` | wrapped monospace, scrollable. Markdown shows as its source text, as the notes picker does today; notes show `notes::note_body`. | notes source; file source for text files; plugin sources | `TEXT_PREVIEW_MAX_BYTES`, then a "truncated" line (`read_text_preview` today; notes use it too instead of reading the whole file). A NUL byte becomes `none` with "Binary file". |
| `image` | fitted to the preview half, aspect kept | file source for image files | decoded and downscaled off the UI thread to the longest edge `ensure_image_preview` uses today, made a named constant. A file over `PREVIEW_IMAGE_MAX_BYTES` (starts at 50 MB) becomes `none` with "Too large to preview". |
| `directory` | file and folder counts, total size, then the first entries by name | file source for folders | stops at `DIR_PREVIEW_CAP` entries and says "and more" |
| `none` | one dim line: the source's reason, else "No preview" | every row with no preview reference: commands, contexts, agents | none |

Loading:
- `LoadPreview` goes to the row's source, which reads on a preview worker, never the UI thread. Results drain in `App::logic` as `Preview { row key, body }`; an image texture is uploaded there too, since `App::logic` has the egui context.
- Previews are cached by row key and modified time, so moving back to a row shows it at once. While a preview loads, the half shows the row title and a dim "Loading".
- A read that has not answered within `PREVIEW_TIMEOUT` (starts at 2 seconds) shows "Preview timed out". A failed read shows "Could not read: <error>" and logs the path and error at `warn`. Neither blocks the results or the next selection.

Showing and hiding:
- Cmd+Y shows or hides the preview in the active view: Finder's Quick Look chord, unbound in `KeyBindings::default`. The choice is remembered per view in `[palette] preview`, written through `config::edit`.
- Open by default: Notes, and Files, because telling `mod.rs` from `mod.rs` is what a file preview is for and loading is off-thread. Closed by default: Everything, Commands and Pinned, whose rows are mostly contexts, agents and commands with no preview, so an open half would sit empty and take width from the list.
- Below `MODAL_WIDTH_MD` the preview is dropped whatever the setting, as `draw_notes_picker` does today.
- The Explorer's Quick Look uses the same kinds, classifier and loader on Cmd+Y, so the chord means preview in every host. Its sidebar stays a 1394 layout choice over the same loader.

Stint 1116 (0.8 Writing Desk, images attached to notes) lands in this `text` kind for notes: inline images through the editor's `preview::image_spans` and `ImageCache`, loaded by the same preview worker. It must not add a second notes preview path; `draw_notes_picker` is deleted by 1391.

## Configuration

Two global tables in `config.toml`. Both are optional; absent keys mean the defaults shown.

```toml
[palette]
views = ["everything", "files", "commands", "notes", "pinned"]  # Tab order
hidden = []                                                      # never reached by Tab
preview = ["notes", "files"]                                     # views with the preview open

[[pins]]
name = "Release"
rows = ["files:/work/plexi/docs/RELEASE.md", "commands:reload_config"]

[[pins]]
name = "Daily"
rows = ["agents:reviewer"]
```

Views:
- `views` is the Tab order. An installed view not listed there (a new plugin view) follows the listed ones in plugin id order. `hidden` removes views from Tab and the chips.
- A chord opens its view even when the view is hidden; the chip shows only while that view is open. To drop a chord, unbind it in `[keybindings]`, which stays the one place chords live. So removing Notes from Tab leaves Cmd+O working, and hiding Everything leaves Cmd+P working.
- An id that names no host view and no installed plugin view, or appears twice, is an `InvalidValue` error naming the valid ids, shown in the reload notice and by `plexi config check`; that key falls back to its default, as the loader does for every invalid key.
- Explorer views (Browse, Find, Actions) are fixed in 0.6. A plugin Picker's views are the plugin's own.

Pinned lists:
- `[[pins]]` is an array of tables so lists keep file order without a map-ordering dependency. `name` is unique; a duplicate is an `InvalidValue` error. `rows` holds row keys in display order.
- "Pin to list" (takes the list name as an inline field) and "Remove from list" are registry commands. They write through `config::edit::edit_config_file` from `App::logic`, as the `ui_scale` flush does, so comments, key order and other tables survive (`toml_edit`). The config watcher then reloads, so the Pinned view reads one path: the loaded config.
- A key its source cannot resolve (file moved, plugin not installed) stays listed as unavailable with the reason, never silently dropped.

Consequences, stated plainly:
- Config is per channel (`config_dir`), so Alpha and Beta keep separate lists. This knowingly departs from the rule in `src/host/AGENTS.md` that user data must not fork per channel.
- Lists are global only. There are no per-context lists, and `[palette]` and `[[pins]]` join the global-only keys, reported as `WorkspaceGlobalOnly` in a workspace config.
- The host writes a file the person may have open in an editor; the last writer wins, as with `ui_scale` today.
- Move pins to user data when any of these becomes true: the person wants a list per context, lists must follow across channels, or agents and plugins write pins. The move adds a `UserDataKind` as `src/host/AGENTS.md` describes, reads `[[pins]]` once, writes the new store and removes the table in the same step, with no second reader left behind.

## Plugin palette views

The API belongs to stint 1395 (0.7 Workbench). What 0.6 fixes so that 1395 does not have to undo it:
- **Declaration.** A plugin declares a palette view in its manifest as its own contribution kind, carrying the same fields as a host view record (id, label, preset, default preview); the host builds the view with a plugin source. `manifest_contract` rejects unknown contribution kinds as "not yet supported" today; 1395 adds this kind and its validation.
- **Id.** Plugin-namespaced like command ids (`<plugin id>.<suffix>`), so it never collides with a host view id, which has no dot. The same id is the source id in row keys and the entry in `[palette]`.
- **Placement.** By default after the host views, in plugin id order. The person reorders or hides it with `[palette] views` and `hidden`. Uninstalling the plugin removes its ids from `[palette]` through `config::edit`; pinned rows from it stay as unavailable.
- **Capability.** Declaring a view needs none; a listing grants nothing (`docs/prm/permissions-broker.md`, command registry). Rows are the plugin's own data. Activating a row runs the plugin's own handler, or a registry command through the broker under the plugin's grants; a plugin row cannot carry a host action.
- **Host guarantees.** The row cap applies and an oversized answer is refused with a typed error. A query unanswered within `PLUGIN_SOURCE_TIMEOUT` shows "<plugin> did not answer" in that view only. A crash or error is `SourceFailed`, shown in that view only. Other views never wait on a plugin: each view queries only its own sources, and plugin views do not feed Everything in 0.6. Plugin previews are `text` only, under the same byte cap, unless 1395 adds kinds.

## Keyboard map

Chords checked in `KeyBindings::default`. Cmd+Shift+P has no binding today; it becomes `open_command_palette_commands`.

| Key | Results step | Fields step |
|---|---|---|
| Cmd+P, Cmd+Shift+P, Cmd+O | open the palette on Everything, Commands, Notes, even when that view is hidden; the same chord closes it | same |
| Cmd+E | open or close the Explorer right split (layout `split_right`) | same |
| Tab, Shift+Tab | next view, previous view | next field, previous field |
| Up, Down, Cmd+K, Cmd+J | move selection | unchanged in a choice field |
| Enter | first action; if it needs input, go to the fields step | next field; run on the last |
| Esc | modal: close. Docked: clear the query, then pass to the host | back to results, query and selection kept |
| Cmd+Y | show or hide the preview; in the Explorer, Quick Look | none |
| Action chords | declared by the view. Notes keeps Cmd+S, Cmd+Backspace, Cmd+Z, copy and cut | none |

- `/` and `>` are ordinary characters. No prefix switches a view. Shift+Enter is reserved for "open in the other placement" (stint 1245, 0.7 Workbench).
- A picker has no bare-key shortcuts. Space, Left, Right, h, j, k and l go to the field. The Explorer's parent folder and the rest become registry commands with chords, picked in the stint 1394 note; Quick Look is Cmd+Y. Parent folder proposal: Cmd+Up, unbound today.
- Cmd+J and Cmd+K are host pane navigation. The modal already wins because `poll_actions` skips `Normal` bindings while an overlay is open. A docked picker needs the typed chord claim from stint 1110: a focused view declares the chords it owns, `poll_actions` skips those bindings, and `dispatch_plugin_key_events` forwards them even while the field has focus. `keyboard_capture` stays all-or-nothing and is not used for this.

## Three hosts, one component

`src/ui/picker.rs` has painters that read a `PickerState` and return `PickerCommand`s: `Field` (chips, `TextField`, fields step), `Results` (`ListRow`s with group headers, scroll to selection, hover and click) and `Preview` (draws the kinds above). Keys are translated in the focus owner's `handle_key`, never in a draw function. The seam for 1395: rows, row keys, action ids, preset names, preview kind names and the row cap contain no host paths, no egui types and no host enums, so they can cross WIT unchanged.

| Host | Shell | Parts | Notes |
|---|---|---|---|
| Palette | `ModalShell::centered`, `FocusKind::CommandPalette` | Field, Results, Preview when the view has it open | Opening keeps the invoking pane and selection as the command target. Views and order from `[palette]`. |
| Explorer | `FileBrowserView` pane, `PickerId::Pane` | Field on top; the Explorer's own table paints the rows | Views Browse (this folder), Find (below this folder), Actions (commands targeting the selection). Preferences move from pane state to an `[explorer]` table in `config.toml`, written through `config::edit` like `ui_scale`. Config is per channel, so Alpha and Beta remember separately. |
| Plugin Picker | a `Picker` node in a plugin pane (stint 1395) | Field, Results, Preview | The plugin sends rows and a preset name and receives `query` and `picked` events. A plugin palette view is the same view record with a plugin source. |

## Deleted

- Stint 1391: `PaletteEntry`, `PaletteAction`, `action_for_palette_entry`, `sort_palette_entries`, `reconcile_palette_selection`, the key block in `draw_command_palette`, `command_palette_handle_key`, `shortcut_label`, the `palette_query`, `palette_selected`, `palette_selected_key` and `palette_scroll_reset` fields; `draw_notes_picker`, `notes_picker_handle_key`, `notes_picker_filtered`, `notes_picker_apply_query`, `notes_picker_preview_text`, `NotesPickerOwner`, and the `notes_picker_query`, `notes_picker_selected`, `notes_picker_last_drawn_selected`, `notes_picker_shown_paths` and `notes_picker_preview` fields. Note delete, undo, copy and cut stay as actions.
- Stints 1151 and 1393: `PaletteCommand`, `PaletteCommandEntry`, `PALETTE_COMMANDS`, `run_palette_selection`, `run_palette_command`.
- Stint 1394: all of `src/file_browser/intent.rs`; in the Explorer `Mode`, the `AppendText`, `Backspace`, `EnterSearch`, `EnterPaletteSlash`, `EnterPaletteCommand` and `TabComplete` arms of `classify_key` and `handle_key`, `draw_search_bar`, `draw_action_palette`, `palette_preview_chip`, `open_palette`, `exit_palette`, `rank_palette_candidates`, `tab_complete_palette`, `submit_palette`, `parse_intent`, `intent_context`, `execute_intent`, `enter_search`, `exit_search`, `draw_rename_modal`, `open_rename_modal`, `confirm_rename_modal`, `cancel_rename_modal`, `toggle_legacy_sort`, `PALETTE_MAX_VISIBLE_CANDIDATES`, `ensure_image_preview`, `ensure_dir_preview`, `ensure_text_preview`, and the `in_search`, `in_palette`, `search_query`, `palette_*`, `search_focus_pending`, `preview_*`, `dir_preview_*` and `text_preview*` fields.

## Build order

Every stint that adds a config key updates `src/config/mod.rs`, `scripts/default-config.toml` and `docs/CONFIG.md` in the same change (`src/config/AGENTS.md`).

| Stint | Lands | Tests that must pass |
|---|---|---|
| 1396 named agent launch (first; edits `command_palette.rs`) | Agent rows keyed by agent name; one `agent open` resolver shared by palette and CLI | `named_agent_focuses_when_running`, `named_agent_resumes_last_chat_when_stopped`, `named_agent_starts_new_when_never_run`, `cli_agent_open_matches_palette`, `ui_tests::palette_lists_defined_agents` |
| 1391 picker core | Reducer, painters, views Everything, Commands, Notes, Cmd+Shift+P; preview worker with `text` and `none` kinds and Cmd+Y; `[palette]` table | `tab_cycles_views_and_shift_tab_goes_back`, `query_survives_view_switch`, `no_key_command_edits_the_query`, `selection_follows_row_key_and_clears_on_removal`, `stale_generation_rows_are_dropped`, `mouse_and_enter_resolve_the_same_action`, `cmd_shift_p_opens_on_commands`, `palette_view_order_comes_from_config`, `unknown_palette_view_names_the_valid_ids`, `hidden_view_still_opens_from_its_chord`, `preview_never_loads_on_the_ui_thread`, `preview_for_a_moved_selection_is_dropped`, `preview_toggle_is_remembered_per_view`, `ui_tests::palette_typing_abc_reads_abc`, `ui_tests::palette_everything_view`, `ui_tests::palette_commands_view`, `ui_tests::palette_notes_view_with_preview` |
| 1392 files and pinned | Shared file index, Files view, files in Everything, Pinned view, `[[pins]]`, `image` and `directory` kinds, the shared file classifier | `file_walk_never_runs_on_the_ui_thread`, `palette_opens_a_file_with_no_explorer_open`, `cached_rows_show_before_the_walk_finishes`, `pinned_list_survives_restart`, `pin_write_keeps_config_comments`, `duplicate_pin_list_name_is_reported`, `unresolvable_pin_shows_as_unavailable`, `oversized_source_answer_is_refused`, `oversized_image_shows_no_preview_with_reason`, `ui_tests::palette_files_view_with_preview`, `ui_tests::palette_pinned_view` |
| 1393 inline fields (after 1151) | Fields step fed by registry argument schemas | `enter_on_command_with_input_moves_to_fields`, `esc_in_fields_returns_with_query_and_selection`, `required_field_blocks_run_and_says_why`, `target_captured_at_enter_is_rechecked_at_run`, `new_file_and_rename_run_from_the_palette`, `ui_tests::palette_inline_fields` |
| 1394 Explorer | Docked picker, right split, config preferences, chord claim, one open resolver, Quick Look on the shared preview loader, parser deleted | `cmd_e_opens_a_right_split`, `explorer_typing_abc_reads_abc`, `hidden_files_choice_survives_restart_and_folder_change`, `explorer_and_notes_open_through_one_resolver`, `focused_picker_owns_cmd_j_k_and_gives_them_back`, `directory_read_never_runs_on_the_ui_thread`, `quick_look_uses_the_shared_preview_loader`, `ui_tests::explorer_browse_view`, `ui_tests::explorer_find_view`, `ui_tests::explorer_actions_view` |
| 1397 agent memory and New chat | "New chat" as an agent row action and a command | `new_chat_archives_the_current_chat`, `archived_chat_reopens_from_history`, `new_chat_reads_the_same_memory_folder` |
| 1398 installs off (separate lane) | Host switch; the palette may open the setting but never flip it | `non_bundled_install_is_refused_with_a_typed_error`, `cli_cannot_bypass_the_install_switch`, `palette_cannot_flip_the_install_switch` |
| 1395 public Picker (0.7 Workbench) | Palette view contribution kind, plugin sources, `PLUGIN_SOURCE_TIMEOUT` | `plugin_view_follows_host_views_by_default`, `slow_plugin_source_never_blocks_other_views`, `failed_plugin_source_shows_only_in_its_view`, `uninstall_removes_plugin_view_ids_from_config` |

## Technical choices

| Choice | Options | Pick |
|---|---|---|
| Where picker state lives | A. Stay on `PlexiApp`: no move, but logic stays inside draw code and needs a UI harness to test. B. Each host owns its own `PickerState`: simple ownership, three places to service. C. `HostModel` keyed by `PickerId`: one reducer, plain Rust tests, state dies with its pane. | C |
| How sources reach the model | A. A trait the reducer calls: direct, but disk and plugin calls enter the model. B. Effects out, rows in: the model stays pure and slow sources are naturally off-thread. C. Sources push whole lists each frame (today): re-ranks every frame. | B |
| Ranking | A. A closure per source: flexible, cannot cross to plugins. B. One ranking everywhere: notes lose title-first order. C. Named presets over one scorer. | C |
| File index | A. A `SearchSession` per picker (Explorer today): two walks of the same root. B. One shared index per root, kept by the host and refreshed in the background on open. C. An index on disk: fast cold start, new format to own. | B |
| Explorer rows | A. The component paints a table too: larger kit surface now. B. The Explorer keeps `draw_details_table` and reads rows and selection from the picker. C. Explorer rows become `ListRow`s: loses columns. | B until 1.0 |
| Action payload | A. Rows carry host enums (`PaletteAction`): cannot cross to plugins. B. Rows carry a row key and action id, resolved by the source; the id is a registry command id once 1151 lands. | B |
| Preview kind | A. The painter guesses from the file name: works only for files, cannot cross to plugins. B. The source sets the kind on the row: one rule for host and plugin rows. | B |
| Tab config shape | A. One exact list: new plugin views stay invisible until added by hand. B. Order plus hidden: new views appear, the person still controls both. C. A table per view: more to read for the same result. | B |

## Decisions for Ian

| # | Decision | Options | My pick |
|---|---|---|---|
| 1 | Confirm pinned lists live in `config.toml` (`[[pins]]`), accepting per-channel lists and no per-context lists until a trigger in Configuration moves them. | `config.toml`; or a new `UserDataKind` | `config.toml` |

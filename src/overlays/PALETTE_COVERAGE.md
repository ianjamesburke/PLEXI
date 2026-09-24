# Palette everyday host-action coverage

This is the V1 coverage boundary proposed by PAL-02. It is an inventory for
review, not a claim that every row is implemented today. Ian's Hand review of a
normal workday is the approval for this boundary. PAL-03 and PAL-04 implement
the gaps; PAL-05 and PAL-06 make their search and execution behavior robust.

## Rule

An **included** task must be searchable in the command palette and either:

- lead to a target/parameter flow that invokes the identified handler; or
- open the identified existing host UI, which owns the target or input flow.

Keyboard shortcuts remain useful accelerators, but do not satisfy discoverability
on their own. A user-defined `run` command is a discrete palette result and
does not make arbitrary CLI verbs palette-covered. The palette must invoke host
behavior through typed actions and existing confirmation paths; it must not
construct an arbitrary shell command from a selected result.

## Catalogs at this boundary

| Catalog | Current role | Boundary consequence |
| --- | --- | --- |
| `PaletteCommand` and `PALETTE_COMMANDS` in `command_palette.rs` | Searchable built-ins: `SplitRight`, `SplitDown`, `OpenConfig`, `OpenQuickNote`, `OpenScratchpad`. Context, app, note, agent, and user-command results are collected separately. | This is the current implementation subset, not the V1 set. New PAL-03/04 rows need an explicit typed result and executor. |
| `Action` and `build_binding_table` in `host::keys` | Existing keyboard-dispatched host behavior, including lifecycle, navigation, view, notes, config, and notification actions. | The actions named below are the primary reuse targets for palette execution. A palette action may need a stable selected target rather than the keybinding's focused target. |
| Clap enums in `cli::args` | Terminal, scripting, authoring, diagnostic, and host-control command surface. | The CLI is an input to the inventory, not an automatic palette backlog. The CLI-only table is deliberate. |

When a row is implemented, keep this document and the corresponding palette
descriptor/executor in sync. The test for a new descriptor should prove its
search aliases and the selected typed action; an end-to-end test should prove
that keyboard selection and pointer selection invoke the same action.

## Included V1 everyday host actions

The requested palette presentation is deliberately task-oriented. “Existing
surface” is an acceptable completion path; it is linked so PAL-04 does not
duplicate an editor or picker merely to claim coverage.

| Everyday task | Required palette result and flow | Existing handler or UI surface | Current state |
| --- | --- | --- | --- |
| Find and focus a context, including a parked context | Search context and pane names; select a stable context/pane target and restore/focus it. | Palette context collection and `jump_to_context`; parked restoration is in `PlexiApp::unpark_context` / `switch_workspace`. | Present after PAL-01; retain as V1 coverage. |
| Create or open a context | `New context` collects name and root; `Open context…` picks a root/path. Advanced child/window forms remain CLI-only. | `Action::NewContext`, `PlexiApp::new_context`, `PlexiApp::new_context_at_path`; CLI `ContextCmd::{New,Open}`. | PAL-03. |
| Rename, park/unpark, or close a selected context | Select a context target; rename collects text; close uses the existing confirmation. | `Action::{RenameContext,ParkContext,CloseContext}`, `PlexiApp::open_context_rename`, `toggle_park_active_context`, `build_context_close_state`. | PAL-03. |
| Move up from a sub-context | `Zoom out of context` acts on the current context. | `Action::ContextZoomOut`, `PlexiApp::zoom_out_of_context`; CLI `ContextCmd::ZoomOut`. | PAL-03. |
| Open a terminal pane or tab; split beside/below | `New terminal` offers the everyday placements (right, down, tab). Existing `Split right` and `Split down` remain aliases. | `Action::{SplitRight,SplitDown,NewTab}`; CLI `PaneCmd::New`. | Split entries present; tab and target/parameter flow are PAL-03. |
| Rename, close, hide/reveal, or zoom a selected pane | Select an explicit pane target. Close must retain the current close confirmation behavior. | `Action::{RenamePane,ClosePane,HidePane,ToggleZoom}`, `PlexiApp::open_rename_for_focused`, `execute_close_pane`. | PAL-03. |
| Move through recent work | `Back` and `Forward` operate on host focus history; `Back` preserves app navigation precedence. | `Action::{NavBackApp,FocusHistoryForward}`, `try_nav_back_focused`, `step_focus_history_back`, `step_focus_history_forward`. | PAL-03. |
| Browse notes or make a quick note | `Browse notes` opens the picker; Quick Note and Scratch Pad stay searchable. | `Action::OpenNotesPicker` / `PlexiApp::open_notes_picker`; `Action::{OpenQuickNote,OpenScratchpad}`. | Quick Note and Scratch Pad present; picker entry is PAL-04. |
| Open or reload the applicable configuration | `Open config` identifies effective scope; `Reload configuration` executes host reload. Raw key editing is not required. | `Action::{OpenConfig,ReloadConfig}`, `PlexiApp::reload_config`; CLI `ConfigCmd::{Edit,Check,Get,List,Set,Reset}`. | Open config present but scope is implicit; PAL-04. |
| Review pending notifications | `Review notifications` opens the existing notification UI. Posting/dismissing a producer notification remains CLI-only. | `Action::ToggleNotificationModal`; `PlexiApp::show_notification_modal`. | PAL-04. |
| Find an agent and focus its pane | Search agent name, title, state, and active tool; select its stable pane target. | Palette `Agent` result and `jump_to_context`; CLI `AgentCmd::Status`. | Present navigation; PAL-05 owns richer matching. |
| Launch an installed app | Search registry apps and select an app result. Arguments and unusual placement are not part of this row. | Palette `App`/`Builtin` results and `launch_palette_app`; CLI `AppCmd::Open`. | Present. |
| Run a named workspace/global command | Search the named command and run it through the existing user-command path. Arguments are a stretch flow. | Palette `UserCommand` result and `run_palette_user_command`; CLI `Commands::Run`. | Present without arguments. |

`ToggleSidebar`, `ToggleShortcuts`, `ToggleMinimap`, pane font changes, directional
navigation, pane swapping/sending, `NewPageRight`, `OpenFileBrowser`, and
`ForceReloadApp` are existing `Action` entries. They are not necessary to meet
the everyday V1 promise. They remain shortcuts or later palette candidates,
rather than silently expanding this approved boundary.

## CLI-only: intentionally not palette parity

These commands are primarily automation, inspection, authoring, administration,
or lifecycle operations outside an already-running palette. They stay
discoverable through CLI help and documentation; a future palette item may offer
terminal handoff or a designed UI, but that is not V1 coverage.

| CLI area | Examples | Why it remains CLI-only |
| --- | --- | --- |
| Pane automation and observation | `pane wait/events/send/key/click/drag/drop/command/state/capture/status/heartbeat/slot` | Script and agent control, synthetic input, or machine-readable inspection; not a human palette action. |
| Advanced pane/context construction | `pane new --agent/--overlay/--window`, `context sub`, `context new --parent --window`, `context push`, `context zoom`, `context describe`, `context set-root` | Multi-parameter orchestration and squad construction need a dedicated design, not a generic command-string form. |
| App development and management | `app init/check/test/render/validate/inspect/package/state/action/trust`, install/uninstall/update/prune/freeze/info | Authoring, package management, trust, or diagnostic workflows need their own parameter and safety UX. |
| Workspace and secrets administration | `workspace init/clean`, every `secret` operation | Setup and credential operations are guarded administrative work. `OpenSecretsManager` is an existing UI surface, not a requirement to mirror all secret verbs. |
| Host/process operations | `host start/stop/status/log/screenshot`, `update`, `uninstall`, `doctor`, `ai`, `demo` | Startup is outside an active palette; the rest are diagnostics, installation, or guided setup. |
| Event and registry plumbing | every `events` command, `registry watch`, descriptor probes, completions | Developer/app transport and shell integration. |
| Notification production | `notify`, `notify dismiss` | A palette may review notifications, but should not impersonate an app/CLI notification producer. |
| Agent integration administration | `agent init/add/update/list/report/hook` | Hook setup and agent scaffolding are not everyday host navigation. `agent status` is represented by palette agent results. |

## Beta or excluded

| Surface | Disposition | Reason |
| --- | --- | --- |
| `routine` | Beta/excluded | `ReleaseFeature::Routines`; no palette promotion. |
| Marketplace: `account`, `app publish/browse/search`, bare marketplace-ID install | Beta/excluded | `ReleaseFeature::Marketplace`; release gate remains authoritative. |
| `events mcp-config` | Beta/excluded | `ReleaseFeature::McpClient`; do not expose an MCP credential from palette. |
| Assistant and mesh capabilities | Excluded | Existing `Action::OpenAssistant` remains release-gated behavior; PAL-02 does not broaden it. |
| Automation pipes, typed pipes, and pane drop workflows | Excluded | App-to-app and binary transport are not human-trust palette actions. This document does not change their transport contracts. |
| Windows-specific keymap, named pipe, or platform bring-up work | Excluded | PAL-02 is a host-palette coverage boundary only. |

## Verification and review

For a PAL-03/04 implementation, add a focused host test for the descriptor and
its stable target/parameter result, then use a seeded command-palette scene and
inspect its PNG as required by `src/testing/TESTING.md`. Hand review should use
normal daily tasks: locate/open work, manage its context and panes, capture and
browse notes, reload config, and review notifications without memorizing a
shortcut. Approval of this list is still awaiting Ian's Hand review.

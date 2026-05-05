<!-- DEV_LOG.md — decision journal for the Plexi project. Newest entries at the top. Records non-obvious choices, abandoned approaches, and root causes so future sessions don't repeat mistakes. -->

## 2026-05-05 — [FIX] Release workflow: use git-cliff --latest instead of custom awk (PR #708 → alpha)

`actions/checkout` defaults to shallow clone (depth=1), so `git tag --sort=-version:refname` only returned the current tag — `prev_tag` was always empty, the awk stop condition never fired, and every GitHub release body contained the full CHANGELOG. Fix: `fetch-depth: 0` on checkout + `git-cliff --latest --strip header` via `taiki-e/install-action`. Deleted the awk/grep tag-hunting block entirely — git-cliff already does this natively.
**Breaks if:** A new GitHub release body contains changelog entries from versions older than the released tag.

## 2026-05-05 — [FIX] Remove idle pane divider stroke; fix hover/drag stroke in light OS mode (PR #707 → alpha)

`egui_tiles` draws divider strokes using `tab_bar_color` (resolves to `visuals.extreme_bg_color`) and hover/drag strokes from `style.visuals.widgets.hovered/active.fg_stroke`. Both derive from `visuals.dark_mode`, which the OS can override to `false` even though we hardcode `dark_mode = true`. In light OS mode: idle stroke went near-white (visible bug); hover/drag stroke went dark (invisible against our dark terminal bg). Fix: override `resize_stroke` in `PlexiBehavior` — `Idle` returns `Stroke::NONE`; `Hovering`/`Dragging` return a 2px stroke using `self.colors.text_primary` (always a light theme color, never derived from `style.visuals`).
**Breaks if:** A visible line appears between panes at rest in macOS Light Mode, OR hovering/dragging a pane boundary shows no stroke in either Light or Dark Mode.
## 2026-05-05 — [CHANGED] Notification modal polish — wider, centered footer chips (PR #704 → alpha)

Widened notification modal from 640→760px (`MODAL_WIDTH_NOTIFY` constant; `MODAL_WIDTH_MD` now used by shortcuts modal which was hardcoded at 640). Added `ui.separator()` above the keyboard hint footer. Switched footer chips from raw `key_chip` calls to `key_combo_list` groups. Fixed chip row centering: `vertical_centered` + `horizontal` does NOT center in egui 0.31 — `Layout::top_down(Align::Center)` expands child rects to full available width at the `next_frame` level, so content paints at x=0 regardless. Fix: pre-measure the row width via `ui.fonts` matching `key_combo_list` spacing constants (INTER_COMBO_GAP=10, TRAILING_GAP=10, KEYCAP_PAD_H=6), then `allocate_ui_with_layout(exact_hint_w, hint_h)` inside `vertical_centered` — `justify_and_align` places the child rect at `center_x − hint_w/2`.
**Breaks if:** Notification modal appears as narrow as the command palette, OR keyboard hint chips appear left-aligned in the footer, OR separator line is missing above footer.

## 2026-05-05 — [FIX] NotifyOption.shortcut: strip reserved nav keys at host ingestion (PR #702 → alpha)

`NotifyOption.shortcut` accepted any string, but the notification overlay reserves `j`/`k` (up/down nav), `h`/`l` (future nav), and `1`–`9` (digit-select). An app sending `shortcut: "j"` produced a silent conflict — the overlay nav fired instead of the option select. Fix: `pub fn is_reserved_shortcut(key: &str) -> bool` in `app_protocol.rs`; the `ShowNotification` dispatch site in `app/mod.rs` now strips reserved shortcuts with a `log::warn!` naming the pane, key, and option label. Python SDK warns at `notify_choice()` call time via `warnings.warn`. Note: `drain_notify_queue` was removed in PR #701 (CLI moved to PLEXI_SOCKET); the only ingestion point remaining is `AppCommand::ShowNotification`.
**Breaks if:** An app calling `emit.notify_choice` with `shortcut: "j"` shows no `[WARN] notify:shortcut` in the log, OR pressing `j` in the notification panel selects the option instead of navigating down.

## 2026-05-05 — [CHANGED] refactor(notify): migrate plexi notify CLI off file queue onto PLEXI_SOCKET (PR #701 → alpha)

`notify_cli` now connects to `PLEXI_SOCKET` and writes a `HostCommand::Notify` JSON line instead of writing to `notify-queue/`. `drain_notify_queue` deleted; notify dispatch moved into `drain_pane_cmd_channel` alongside `set_pane_title` and `spawn_pane`. `HostCommand::Notify` gains a `response_file: Option<String>` field (CLI-only, `skip_serializing_if = "Option::is_none"`) to carry the response path for the blocking `--choice` path. One-time cleanup of `config_dir/notify-queue/` at startup. Trade-off: `plexi notify` now fails immediately if Plexi is not running — the old file queue silently buffered offline sends for pickup on next launch; the socket does not.
**Breaks if:** `plexi notify --title Foo --body Bar` inside a terminal pane fails with "PLEXI_SOCKET is not set", OR `plexi notify --title Q --choice yes:Yes --choice no:No` does not block and return the chosen key.
## 2026-05-05 — [CHANGED] Background apps: tick parked processes + command palette indicator (PR #700 → alpha)

Manifest `background = true`, `is_background()`, `background_tick()`, park-on-close (sends `Suspend`), and unpark-on-open (sends `Resume`) were all already wired. The missing piece: `drain_all_app_commands` only iterated `context.panes` — parked apps in `self.background_apps` were never ticked, so timers stalled and notifications never fired after the pane was closed. Fix: after the context-pane loop, iterate `self.background_apps`, call `background_tick()` + `take_pending_commands()`, and route `ShowNotification` to `deferred` with `sender_pane_id = 0` (no live pane sentinel). Added `running_in_background: bool` to `PaletteEntry::App` in `command_palette.rs` — populated from `self.background_apps.contains_key(&id)`, renders a dim `bg` badge. `stand-up-reminder` in `examples/` is the POC (already had `[launch] background = true`).
**Breaks if:** Closing the `stand-up-reminder` pane and waiting for its timer interval fires no notification, OR Cmd+P while the app is parked shows no `bg` badge next to "Stand Up Reminder".

## 2026-05-05 — [CHANGED] Terminal copy-mode — keyboard-driven scrollback selection (PR #696 → alpha)

Adds tmux-style copy-mode to terminal panes. Entry: `Cmd+Y` (handled entirely in `deps/egui_term/src/view.rs` — no host keys.rs changes needed). All key events are intercepted in `process_copy_mode_event` when `state.copy_mode.is_some()`; nothing reaches the PTY. Navigation: h/j/k/l + arrows, PgUp/PgDn, g/G. Selection: v (simple), V (line-wise) via existing `SelectStart`/`SelectUpdate`; y yanks and exits; Esc cancels selection then exits. `[COPY]` badge and block cursor overlay rendered in `show()` after the main grid pass. Alt-screen guard: entry silently no-ops when `ALT_SCREEN` is set. Added `BackendCommand::ClearSelection` for clean exit. Entry key was originally `Cmd+Shift+[` (spec) → `Cmd+Shift+C` (v1) → `Cmd+Y` (final) to avoid conflicts with macOS system shortcuts and existing Plexi bindings.
**Breaks if:** `Cmd+Y` in a normal terminal pane does not show the `[COPY]` badge and block cursor, OR `y` after a selection does not copy text to clipboard, OR pressing navigation keys in copy-mode writes characters to the PTY.

## 2026-05-05 — [CHANGED] git-app: gh-powered merge arcs + edge routing fix (PR #694 → alpha)

`git_log.py`: added `fetch_pr_data()` (runs `gh pr list --state all --json number,headRefOid,state,mergeCommit`) and `annotate_commits_with_prs()` — matches squash commits via `mergeCommit.oid` (not subject-parsing), sets `merge_source_hash = headRefOid`, and marks open PR feature tips with `pr_state = "OPEN"`. `commit_graph.py`: squash commits render as ring nodes; arcs drawn when the feature tip is still in the local git graph (within the ~2-week GC window); open PR tips get green `#NNN` badges instead of muted grey. Edge routing fixed: horizontal elbow now routes through the gutter just below the source node (`y_route = y1 + ROW_H/2`) rather than the midspan midpoint, preventing the edge from passing through intermediate row node centers on odd-row-span edges.
**Breaks if:** Open PR feature tips show grey `#NNN` badges instead of green, OR squash commits on alpha that have merged PRs don't render as ring nodes (hollow circles).

## 2026-05-05 — [CHANGED] DrawCommand::SpawnPane — terminal and app pane spawning from apps (PR #692 → alpha)

Rust host was already fully wired on alpha (protocol types, capability gate, routing, dispatch). Added the missing completions: serde round-trip tests for `SpawnPane`/`PaneSpawned`/`PaneSpawnError` in `app_protocol.rs`; `ctx.spawn_pane()` proxy in `_render_context.py`; `examples/spawn-pane-poc/` POC with two buttons. Bug found during testing: `AppCommand::SpawnPane` dispatch called `launch_app_by_id_with_layout("terminal", ...)` — `"terminal"` is a builtin pane type, not in the registry, so the spawn silently failed. Fixed by routing `type_id == "terminal"` to `split_focused()` instead. Second bug: `split_focused` has inverted `LinearDir` vs `split_with_new_pane` — `split_focused(false)` creates side-by-side (RIGHT) not stacked (BELOW). Mapped `split_h`/`split_above` → `split_focused(true)` and `split_v` → `split_focused(false)` to match expected visual directions.
**Breaks if:** Opening spawn-pane-poc and clicking "Spawn terminal (split_h)" creates a pane to the right instead of below, or clicking "Spawn snake (split_v)" creates a pane below instead of to the right.

## 2026-05-05 — [CHANGED] Shortcuts modal redesign — sections, divider, full coverage, contact footer (PR #693 → alpha)

Replaced the flat two-column grid with five labeled sections (LAYOUT, NAVIGATION left; APPS & TOOLS, TERMINAL, SETTINGS right). Added `ui.separator()` between columns for a vertical divider. Centered title via `ui.vertical_centered`. Added 10 missing shortcuts confirmed in `keys.rs`. Fixed stale labels (⌘N → "New window right", ⌘⇧HJKL → "Navigate windows"). Extracted `draw_contact_footer` free function shared by both the shortcuts modal and the welcome screen. The coffee emoji was updated from ☕ to ❤️ in the process. Column divider uses egui's `ui.separator()` inside a `ui.horizontal` — draws perpendicular to layout direction, no custom painting needed.
**Breaks if:** ⌘/ opens a blank or broken overlay, OR the welcome screen contact footer disappears, OR any pre-existing shortcut (⌘P, ⌘HJKL, ⌘]/[) no longer appears in the modal.

## 2026-05-05 — [CHANGED] Cmd+P palette ranks active context first (PR #691 → alpha)

Cmd+P entries previously sorted by a single recency key, so a pane visited 10 minutes ago in another context could outrank a pane opened seconds ago in the active context. Changed `rank_of` in `src/command_palette.rs` to return a two-tier `(tier, recency)` key: tier 0 if the entry's window belongs to the active context (matches `windows[active_window].context_id`), tier 1 otherwise. Recency calculation unchanged. Mirrors macOS Cmd+Tab behavior — current app's windows first.
**Breaks if:** Open contexts A and B, focus a pane in A, touch a pane in B, switch back to A, hit Cmd+P → any B pane appears above any A pane.

## 2026-05-05 — [CHANGED] Tier 3 --help crawl fallback descriptor renderer (PR #685 → alpha)

Added `src/cli_crawl.rs` — the third tier of CLI descriptor resolution. When a CLI has no `--plexi` native support (Tier 1) and no embedded registry entry (Tier 2), `plexi descriptor probe` now runs `<cli> --help`, parses the command list, and synthesises a `PlexiDescriptor`. Results cached under `~/.plexi-<channel>/cache/descriptors/<cli>.json`. Added `Serialize` to all `PlexiDescriptor` types for cache write. Added `--no-crawl` flag and `SummarySource::Crawled { from_cache }` badge in probe output. Two parsing strategies: (1) recognized section headers (COMMANDS, SUBCOMMANDS, CORE COMMANDS) for tools like `gh`/`cargo`; (2) broad 3-space-indent scan fallback for prose-header CLIs like `git`. The fallback was discovered necessary during testing — `git --help` uses section labels like "start a working area" rather than any standard COMMANDS keyword.
**Breaks if:** `plexi descriptor probe git` errors "could not extract any commands" instead of listing ≥8 git subcommands with "(inferred from --help)" badge.

## 2026-05-05 — [FIX] Offload audio/MIDI device enumeration to background thread (PR #688 → alpha)

`ListAudioDevices` and `ListMidiDevices` were dispatched synchronously on the UI thread. cpal's CoreAudio enumeration blocks while scanning for Bluetooth devices — logs showed 10+ second UI freezes immediately after `audio-recorder` launched. Fix: clone `Arc<dyn AudioDevice/MidiDevice>`, spawn a background thread, send the result back via `http_tx` (same async channel used by `HttpRequest`). `log::info!` traces confirm thread completion. A prior attempt on `temp-fix-audio-enum-blocking` had the identical approach — ported forward cleanly.
**Breaks if:** audio-recorder shows "Loading devices..." indefinitely, or `grep "ListAudioDevices complete" ~/.plexi-alpha/plexi.log` returns nothing after launch.

## 2026-05-05 — [CHANGED] PLEXI_SOCKET listener + plexi pane set-title (PR #686 → alpha)

Binds a `UnixListener` on `config_dir/notify.sock` at startup (removing any stale socket first). An accept thread deserializes newline-delimited `HostCommand` JSON and feeds an `mpsc` channel drained every egui frame. Adds `SetPaneTitle { pane_id, name }` to `HostCommand` (serde tag `"type": "set_pane_title"`); dispatch mutates `TerminalPane.name` directly — the per-frame `pane_names` snapshot in `PlexiBehavior` is built from this field, so the tab label updates on the next frame. `SpawnPane` over socket routes to `launch_app_by_id_with_layout`; unsupported kinds warn-and-drop. `plexi pane set-title <name>` reads `PLEXI_PANE_ID` + `PLEXI_SOCKET` from env, connects, writes one JSON line, closes. Fails fast with a clear error when run outside a Plexi pane. `SetPaneTitle` over PGAP logs warn and drops (protocol error). `new_for_test()` now returns `(Self, Sender<HostCommand>)` so tests can inject IPC commands.
**Breaks if:** Running `plexi pane set-title "x"` inside a terminal pane fails to connect (socket not bound), OR the tab title doesn't update within one frame, OR running outside Plexi exits 0.

## 2026-05-05 — [CHANGED] chat-poc: copy buttons, in-flight overlay, tool docstring (PR #684 → alpha)

Per-bubble `copy` buttons rendered as a post-render overlay pass clipped to the scroll region — positions computed by measuring each ChatBubble height and tracing the Column layout constants (`SPACE_SM/XL/MD`, `BAND_H`, `DIVIDER` margins). `on_click` hit-tests stored rects; `emit.copy_to_clipboard` writes to clipboard; 1.5s `✓` flash confirms. In-flight dim overlay over the TextInput (8-digit hex alpha `#1e1e2e99`). Cross-pane tools from `tool-poc` already auto-injected by the broker at dispatch time — no app-side change needed; updated `ai_query` docstring to remove the stale "reserved for v3.4" note. Streaming deferred (stretch goal, needs host protocol).
**Breaks if:** Clicking the `copy` label on an assistant bubble does nothing, or opening chat-poc alongside tool-poc and asking the model to "increment the counter" fails to update the counter app.

## 2026-05-05 — [FIX] App launch from welcome screen seeds tree root (PR #683 → alpha)

`open_process_app_pane` and `open_builtin_app_pane` both ended the split path by calling `split_with_new_pane`, which early-returns `None` when `focused_pane` is `None`. Result: launching from the welcome screen inserted the pane into the `panes` map but added no tile to the tree — a leaked invisible process with no UI. Fix: after inserting the pane, check `focused_pane.is_none()`. If true, install the new pane's tile as `tree.root` and set `focused_pane` directly, skipping the split entirely. Also added `log::warn!` to the overlay bail-out paths (previously silent returns) and `log::info!` at successful entry points. HostHarness test added in `pane_ops/create.rs`.
**Breaks if:** Opening the command palette on the welcome screen and launching any app (e.g. snake) leaves the welcome screen unchanged with no pane appearing.

## 2026-05-05 — [CHANGED] Notification timeout, tombstone, required-pinned (PR #679 → alpha)

Added `timeout_secs` and `on_dismiss` to `HostCommand::Notify` and `PendingNotification`. A 1Hz tick in `update()` auto-dismisses expired notifications and delivers `PlexiEvent::NotifyAction` with `value = on_dismiss` (defaults to `"timeout"`). Added `tombstoned: bool` to `PendingNotification` — set by `tombstone_pane_notifications()` when a pane closes; tombstoned cards stay in the panel with a dim "Source ended" label and Dismiss-only button. `sorted_notification_ids()` now sorts `(required DESC, priority DESC, arrival ASC)` so required/BlockedOnUser notifications always float to top. Python SDK `notify()`, `notify_choice()`, `notify_input()`, `notify_and_wait()` and all `RenderContext` proxies accept the new params. Note: CLI `--timeout` is still a process-level polling timeout only; host-side auto-dismiss via CLI is tracked in #682.
**Breaks if:** A `timeout_secs=5` notification from the notification-tester (`x` key) does not auto-vanish after ~5 seconds, OR closing an app pane removes its notification entirely instead of showing "Source ended".

## 2026-05-05 — [CHANGED] Secrets inject toggle in add form + build_env logging (PR #670 → alpha)

`commit_add()` hardcoded `inject: false` — `store_secret()` calls `index_add()` internally which always writes `inject: false` for new entries, so the optimistic in-memory `→env` badge appeared but `build_env()` saw nothing to inject. Fix: call `toggle_inject_secret` immediately after `store_secret` succeeds when `new_inject` is true. Also added `new_inject: bool` field to the add form with checkbox + help text. Added `log::info!` in `build_env()` when a secret is injected so the event is visible in the log drain.
**Breaks if:** Adding a secret with Inject checked and opening a new terminal pane shows empty for `echo $KEY`, or the `→env` badge doesn't appear on the saved entry.

## 2026-05-05 — [CHANGED] Show welcome screen instead of deleting context when last pane is closed (PR #678 → alpha)

Closing the last pane in a context no longer removes the context. `execute_close_pane` now checks the page count for the context before calling `delete_window` — it only deletes the page if there are sibling pages. When a context is down to its sole page, the empty window is kept and the existing `panes.is_empty()` render branch shows the welcome screen. Multi-page close behavior and explicit sidebar × deletion are unchanged.
**Breaks if:** Closing the last pane in a context causes the context to disappear from the sidebar instead of showing the welcome screen.

## 2026-05-05 — [FIX] Normalize egui arrow key names in SDK; fix stale audio capture session (PR #677 → alpha)

Two fixes bundled. (1) `_normalize_key()` in `_app.py` maps egui Debug-format key names (`"ArrowLeft"` etc.) to documented SDK names (`"left"` etc.) before dispatching `on_key` — both forms now work, so agents using either don't silently break. All 14 example apps updated to canonical names. (2) `start_audio_capture` in `routing.rs` now cleans up stale sessions: when a pipe breaks mid-recording (EBADF on Python side / Broken Pipe on Rust side), the host's `audio_capture_sessions` entry was never removed, permanently blocking re-use of the same `pipe_id`. Fix: on a new `AudioCapture` command for an already-registered `pipe_id`, drop the old `CaptureSession`, close the pipe, clear the peak meter entry, then proceed with the new capture.
**Breaks if:** After stopping and restarting recording in audio-recorder, the VU meter stays at 0 and the log shows "pipe connect timed out" on the second attempt.

## 2026-05-05 — [CHANGED] Audio playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (PR #673 → alpha)

`AudioPlay` routing fully wired via `rodio` — sessions keyed on source path, state machine for playing/paused/stopped. `AudioMeter` `RenderCommand` now renders a green→yellow→red amplitude bar; peaks tracked in the capture `FrameSink` via `Arc<Mutex<HashMap<String, f32>>>` and passed to `render_draw_commands` as a snapshot. Python SDK gains `emit.list_audio_devices()` (mirrors `list_midi_devices`) with `AudioDeviceInfo`/`AudioDeviceList` protocol types. New `examples/audio-recorder/` POC: device dropdown, record/stop, live peak meter, save-to-WAV via stdlib `wave`. Arrow key handling: canvas apps receive `"ArrowLeft"`/`"ArrowRight"` (egui Debug format), not `"left"`/`"right"` as the SDK docstring misleadingly states — snake.py and tetris.py already used the correct form.
**Breaks if:** Opening the audio-recorder app shows "Device list failed" in status, OR the peak meter stays at 0.0 during mic capture, OR `recording.wav` after saving is 44 bytes (empty header).

## 2026-05-05 — [CHANGED] StreamProcess / CancelProcess / StreamChunk / StreamEnd (PR #671 → alpha)

Added four PGAP v3 streaming variants deferred from #78 (Canvas Terminal Binding Primitives). Host spawns `sh -c <command>` with stdout/stderr piped; a cancel-flag-guarded reader thread delivers `StreamChunk` events via the existing `http_tx` async channel. `CancelProcess` sets the cancel flag, sends SIGTERM, then escalates to SIGKILL after 1s on a background thread. `StreamEnd` is always delivered (on natural exit, cancel, or capability denial) so SDK iterators always unblock. `StreamChannel::Structured` is reserved on the wire but emits stdout bytes in v1. Capability gate: `terminal.bindings` on both new commands.
**Breaks if:** A Canvas app with `terminal.bindings` emits `stream_process` and receives no `stream_chunk` events, or `cancel_process` does not deliver `stream_end` within 1s.

## 2026-05-04 — [FIX] Revert broken sidebar double-click, restore single-click (PR #659 → alpha)

PR #657 added `ui.interact(full_row, Sense::click())` after `ui.interact(content_zone, Sense::click_and_drag())`. In egui, overlapping widgets with click sense compete — the last-registered one steals the event. The full-row widget won, so `drag_response.clicked()` always returned false and single-click context switching broke. Reverted to pre-#657 state. Proper fix is the `SidebarAction` enum refactor (issue #660), which uses one `interact()` on the full row and classifies actions by pointer position — no competing widgets possible.
**Breaks if:** Single-clicking a context row in the sidebar fails to switch context.

## 2026-05-04 — [FIX] Remove deleted agent apps from core pack list (PR #656 → alpha)

`agent-tester`, `agent-coordinator`, and `agent-worker` were removed from `examples/` in PR #628 but `packs/core.toml` still listed them as `local:` sources. `core_pack_applies_only_when_apps_dir_empty` panicked because the local source no longer exists. Fix: delete the three stale `[[app]]` blocks from `core.toml`.
**Breaks if:** `cargo test core_pack` still fails with `core pack entry 'agent-tester' did not install`.

## 2026-05-04 — [FIX] Widen context rename double-click hit box to full row (PR #657 → alpha)

`drag_response` in `sidebar_row.rs` is registered on `self.layout.content` — the row minus the 30px action zone. Double-clicking the rightmost 30px didn't trigger rename. Fix: add a separate `Sense::click()` interaction on `self.layout.full` and OR its `double_clicked()` into `primary_double_clicked`. Single-click and drag behavior unchanged.
**Breaks if:** Double-clicking the right edge of a context row (near where the × appears on hover) fails to open the rename input.

## 2026-05-04 — [FIX] Move gen_schema to workspace, restore PR build display name + icon (PR #655 → alpha)

PR #634 added `gen_schema` as a second `[[bin]]`, forcing `install.sh` to use `cargo bundle --release --bin plexi`. The `--bin` flag tells cargo-bundle to use the binary name ("plexi") as the bundle name, ignoring `[package.metadata.bundle] name = "Plexi Alpha"`. Result: all builds (alpha, PR) produced `plexi.app` (lowercase, no icon). Fix: move `gen_schema` to `tools/gen_schema/` as a Cargo workspace member. Main package is single-binary again; `cargo bundle --release` (no `--bin`) picks up the metadata name and `app_src="${display}.app"` resolves correctly.
**Breaks if:** `just pr-install <N>` creates `plexi.app` instead of `Plexi PR<N>.app`, or Spotlight shows the default macOS icon.

## 2026-05-04 — [CHANGED] Typed SDK command models, py.typed, and plexi validate (PR #651 → alpha)

Added 6 typed dataclasses to `sdk/python/plexi_sdk/_types.py` (`TextCommand`, `RectCommand`, `BadgeCommand`, `TextInputSpec`, `ShortcutPair`, `NotifyOption`) — each validates at `__post_init__` (bad align values, negative geometry, empty required string fields). Added `py.typed` PEP 561 marker and mypy config in `pyproject.toml`. New `sdk-typecheck.yml` CI workflow runs mypy on SDK changes. Added `plexi validate <path>` CLI: reads `manifest.toml`, checks required fields (`id`, `name`, `version`, `entry`), verifies entry file exists, runs Python AST syntax check on `.py` entries. Capability validation warns on unknown capability strings. The `_render_context.py` method API is unchanged — existing app code requires no updates.
**Breaks if:** `plexi validate examples/chat-poc` does not exit 0 with a ✓ message, or `from plexi_sdk import TextCommand; TextCommand(0, 0, "", 14, "#fff", align="bad")` does not raise `ValueError`.

## 2026-05-04 — [FIX] New windows/pages inherit cwd from focused pane (PR #652 → alpha)

`new_context` (Cmd-N) and `create_page_at` (new page) both hardcoded `dirs::home_dir()`. Both now call `get_focused_pane_cwd()` on the active window's focused tile and fall back to home only when cwd is unavailable. `split_focused` (Cmd-Shift-J) was already correct — no change needed.
**Breaks if:** Cmd-N opens a new context in `~` even when the focused pane is in a project subdirectory.

## 2026-05-04 — [CHANGED] Once-a-day update check with toolbar badge + copy_button widget (PR #648 → alpha)

Background thread spawned at startup hits GitHub releases API, caches result in `~/.plexi-<channel>/update_cache.json` for 24h. When a newer version is found, the version label in the toolbar turns accent-colored with a `↑` prefix. Clicking it opens the changelog as usual; the top of the changelog shows a tinted banner with the new version and a 📋 copy button for `plexi update`. The `📋 → ✓` copy pattern was extracted as `copy_button()` in `src/widgets.rs` and back-applied to the welcome page email copy. Cache uses Unix timestamp; `checked_at: 0` forces a re-fetch, a fresh timestamp skips the network.
**Breaks if:** Version label stays dim with no `↑` prefix after injecting a fresh cache with a higher version, OR the 📋 button in the changelog banner doesn't flip to ✓ on click.

## 2026-05-04 — [CHANGED] Split plexi_sdk/__init__.py into focused modules (PR #649 → alpha)

Broke the 2980-line `__init__.py` monolith into `_constants.py`, `_types.py`, `_emitter.py`, `_pipe.py`, `_render_context.py`, `_app.py`. `__init__.py` is now a thin explicit re-export layer. Circular deps resolved via `TYPE_CHECKING` guards — `Emitter` and `Pipe` forward-ref `App` as strings only. `_emit` is the leaf export (imported by `_pipe` and `_app`). No public API changes; all example app imports unchanged.
**Breaks if:** Any canvas app pane shows a blank screen or `ImportError` on launch.

## 2026-05-04 — [CHANGED] README overhaul — composability lede, Features + Roadmap (PR #647 → alpha)

Rewrote the lede around the Unix composability angle ("one binary, everything speaks one protocol, pipe output between processes"). Removed: keyboard shortcuts table (in-app `Cmd+/` is the source of truth and won't drift), "What's in v3" heading (temporal, meaningless to a new reader), "agents all install into it" (agents don't install — they run in panes), "Nothing leaks between workspaces" (vacuous). Added: Features section (present-tense, accurate), Roadmap section (6 near-term items, plain bullets, no issue links). Bundled Python demoted from headline feature to impl detail inside the app runtime bullet.
**Breaks if:** README on GitHub still shows "What's in v3" heading or the keyboard shortcuts table.

## 2026-05-04 — [CHANGED] Keyboard pane swap Cmd+Ctrl+HJKL + edge pulse (PR #646 → alpha)

Swaps `PaneId` values at two `Tile::Pane` leaf nodes in the egui_tiles tree — tree structure and rects stay fixed, only pane content moves. Focus follows the swapped pane to its new tile. Boundary press produces a 120ms accent edge glow on the blocked side. Swap shows a 160ms ease-out cubic rect overlay. Issue #517 (`feat(media): video decode`) was found fully implemented on alpha already — closed without new code.
**Breaks if:** `Cmd+Ctrl+L` on a 2-pane split does nothing, or app crashes/shows blank panes after a swap.

## 2026-05-04 — [FIX] BSD awk compat in release notes extraction (PR #643 → alpha)

`match($0, /regex/, arr)` three-argument form is GNU awk only — macOS CI runners use BSD awk and fail with `syntax error`. Replaced with `index($0, "[" stop "]") > 0` which is POSIX/BSD compatible.
**Breaks if:** "Extract release notes" step exits with `awk: syntax error at source line 4`.

## 2026-05-04 — [FIX] release workflow --bin plexi for cargo bundle (PR #642 → alpha)

`cargo bundle --release` without `--bin` uses the display name from `[package]` metadata ("Plexi Alpha"), producing `Plexi Alpha.app` instead of `plexi.app`. Codesign and zip steps expected `plexi.app` and failed. Added `--bin plexi` to force the binary name as the bundle name.
**Breaks if:** release workflow codesign step fails with `No such file or directory`.

## 2026-05-04 — [CHANGED] promote main chains alpha→beta→main (PR #641 → alpha)

`just promote main` from alpha now auto-promotes alpha→beta first, then beta→main. One command from alpha releases all the way to main. From beta it still does beta→main only.
**Breaks if:** `just promote main` from alpha pushes alpha directly to main without going through beta.

## 2026-05-04 — [FIX] release workflow bundle path (PR #640 → alpha)

`cargo bundle` with multiple `[[bin]]` entries in `Cargo.toml` names the bundle after the binary (`plexi`), not the display name (`Plexi`). Release workflow was referencing `Plexi.app` → `No such file or directory`. Fixed codesign and zip steps to use `plexi.app`.
**Breaks if:** release workflow fails at codesign or zip step with `No such file or directory`.

## 2026-05-04 — [FIX] promote beta→main idempotent tag (PR #639 → alpha)

`just promote main` crashed with `fatal: tag already exists` on re-run. Now checks local and remote tag existence before creating/pushing. Safe to run twice.
**Breaks if:** `just promote main` on a fresh release silently skips tag creation.

## 2026-05-04 — [CHANGED] release pipeline cleanup — single-responsibility commands (PR #638 → alpha)

Removed `bump-and-install`, `bump-alpha`, `release`, `release-version`. `just bump [patch|minor|major]` is now the single bump command (calls `release-version.sh`, git-cliff, commits). `just promote` is now a clean channel push — no bump, no changelog. Post-merge cycle is `just bump && just install`. Deleted `scripts/bump-alpha.sh`, `scripts/bump.sh`, `scripts/release.sh`. Ship skill and CLAUDE.md updated.
**Breaks if:** `just promote beta` bumps the version or modifies CHANGELOG (should be a clean push only).

## 2026-05-04 — [CHANGED] git-cliff changelog + release-version command (PR #637 → alpha)

Replaced manual awk changelog generation in `promote.sh` with git-cliff. Added `cliff.toml` (flat list, first-line-only to strip squash bodies, skips bump/promote/DEV_LOG/merge noise). Added `scripts/release-version.sh` and `just release-version [patch|minor|major]` for explicit semantic version bumps before promote. `just promote` still auto-bumps patch but now calls git-cliff when available. Added `jc` shell function to dotfiles (runs `just <recipe>`; on failure opens Claude with error context); renamed old `jc='just --choose'` alias to `jj`.
**Breaks if:** `just promote beta` changelog section is empty or shows raw squash-merge bodies instead of single-line entries.

## 2026-05-04 — [CHANGED] triage→triage-issues rename + touches/clarification_needed front matter + sprint-plan batch skill (PR #636 → alpha)

Renamed `.claude/skills/triage/` to `.claude/skills/triage-issues/` and updated the `name` field — `/triage` no longer resolves. Extended the triage skill's Step 3 (LOC estimation) to record a `touches` list (top-level files/dirs) in front matter, and Step 8 (Actionability) to emit a `clarification_needed` list of concrete open questions. Both fields are written to issue front matter and surfaced in the Step 11 triage comment. New `/sprint-plan` skill does a read-only two-pass batch scan: (1) clarification sweep — all issues with open questions or labeling gaps; (2) execution plan — topological sort by `depends_on` + priority, grouped into parallel lanes based on `touches` overlap. Motivation: issue #625 slipped through without a `ready` label because it had no `blocked` label and no one added `ready` after scoping — `touches`/`clarification_needed` + `sprint-plan` make this class of gap visible.

**Breaks if:** `/triage-issues` fails to load (skill name mismatch), or `/sprint-plan` is not found in the skills list.

## 2026-05-04 — [CHANGED] Rust-owned canonical PGAP schema + generated Python protocol models (PR #634 → alpha)

Added `JsonSchema` derives (via `schemars`, already in Cargo.toml) to all 23 protocol types in `src/app_protocol.rs`. New `src/bin/gen_schema.rs` binary emits a combined JSON Schema for `PlexiEvent`, `RenderCommand`, `HostCommand`, `ControlCommand` to stdout; checked-in as `sdk/protocol/pgap.schema.json`. `sdk/protocol/pgap.version.json` declares `{"protocol": "pgap/3", "version": 3}`. `tools/gen_protocol_py.py` reads the schema and generates `sdk/python/plexi_sdk/_protocol.py` with `PROTOCOL_VERSION`, `AiResponse`, `MidiPortInfo`, `MidiDeviceList` dataclasses, replacing the handwritten mirrors in `__init__.py`. `just gen-schema` regenerates both artifacts; `.github/workflows/check-protocol.yml` CI fails if `src/app_protocol.rs` changes without regenerating the schema.

Required adding `src/lib.rs` to expose `app_protocol` as a library target (with stub `midi`/`audio`/`video` modules) so `gen_schema.rs` can depend on `plexi::app_protocol` without pulling in the full GUI dependency tree. `install.sh` changed to `cargo bundle --release --bin plexi` with `app_src="target/release/bundle/osx/plexi.app"` — cargo-bundle assigns the metadata bundle name to the first `[[bin]]` it encounters, so without `--bin` the wrong binary (gen_schema) ends up in the app bundle.

**Breaks if:** Any canvas app pane shows a blank screen or ImportError on launch (would mean `_protocol.py` broke the SDK import chain). Check: open todo app — if it renders, SDK is intact.

## 2026-05-03 — [CHANGED] plexi install <id> — registry-aware bare app ID shorthand (PR #633 → alpha)

`install_cli` in `src/cli.rs` now detects bare IDs (no `:`, `/`, or `@`) and resolves them via `https://raw.githubusercontent.com/ianjamesburke/plexi-registry/main/registry.json` before calling `parse_source_spec`. Registry entries in `ianjamesburke/PLEXI` with a `path` field resolve to `local:<dir>` (uses the bundled copy, no clone). Third-party repos resolve to `github:owner/repo`. Unknown IDs print a helpful error with `plexi app list` and `plexiapp.com/apps`. No changes to `install.rs`, `main.rs`, or `packs.rs`.

**Breaks if:** `plexi install snake` prints a source-scheme error ("unknown source scheme 'snake'") instead of installing the app.

## 2026-05-03 — [CHANGED] Zoom overlay at 88% opacity — scrim bleed-through (PR #629 → alpha)

`child_ui.set_opacity(0.88)` added immediately before the `egui::Frame` in the zoom overlay path of `src/app/mod.rs`. Prior attempts at 0.99 and 0.95 were visually imperceptible on dark themes; 0.88 is the first value that makes the scrim bleed-through visible. The frame fill alpha approach was also tried (Attempt 1) and failed — the terminal renderer paints fully-opaque cell backgrounds on top, overriding any fill alpha. `set_opacity` is the only path that propagates through the full paint chain.

**Breaks if:** Zooming any pane shows the overlay frame as fully opaque with no visible bleed-through from the scrim, or the overlay is distractingly transparent.

## 2026-05-03 — [CHANGED] Batch Python frame output; remove frame.clone() hot-path (PR #630 → alpha)

Python: `RenderContext` now buffers all draw commands in `self._buf` instead of calling `sys.stdout.flush()` per command. `frame_done()` writes the entire frame (all draw commands + the sentinel) in one `sys.stdout.write()` + `flush()`. A 150-command frame goes from 151 flushes to 1. Out-of-frame signals (`log`, `notify`, `status_summary`) still flush immediately. `measure_text` flushes the buffer before sending so the host can process prior commands in order.

Rust: removed `let frame_clone = self.frame.clone()` from `ProcessApp::ui()`. `render_text_inputs` no longer accepts a `frame` parameter — it reads `self.frame` directly, which resolves the borrow-checker conflict that previously required the clone.

**Breaks if:** Any canvas app pane renders blank or partially after merge, or text inputs / scroll regions stop responding.

## 2026-05-03 — [CHANGED] Remove stale iq.query POC apps, fix capability hint (PR #628 → alpha)

Deleted `examples/agent-tester`, `agent-worker`, `agent-coordinator` — POC apps for closed issues #338 and #286 that still declared `iq.query` (renamed to `ai.query`), generating 3 WARN skip lines on every startup. Added `Capability::all_str_values()` and used it in the `app_registry` error message so the valid-values hint derives from the enum rather than a hardcoded string. Installed copies in `~/.plexi-alpha/apps/` still need manual removal to silence the WARNs on the running build.

**Breaks if:** Startup log still shows `skipping ... unknown capability: 'iq.query'` WARNs after manually deleting the three app dirs from `~/.plexi-alpha/apps/`.

## 2026-05-03 — [CHANGED] Cmd+J / Cmd+K palette navigation aliases (PR #620 → alpha)

`consume_key` checks for `COMMAND+J` (down) and `COMMAND+K` (up) added alongside the existing `ArrowDown` / `ArrowUp` handlers in `src/command_palette.rs`. Both use an `||` short-circuit so arrow keys are unaffected. Bounds checks are identical to the arrow-key path.

**Breaks if:** Cmd+J / Cmd+K do nothing while the command palette is open, or arrow-key navigation stops working.

## 2026-05-03 — [CHANGED] Zoom overlay inset 5px → 10px (PR #618 → alpha)

Wider gap makes the zoomed state read as an intentional overlay rather than nearly full-bleed. One-line change: `let inset = 5.0` → `10.0` in the zoom overlay block of `src/app/mod.rs`. `zoom_rect = panel_rect.shrink(inset)` propagates the change uniformly to all four sides.

Opacity (issue #572) was attempted in the same PR (`child_ui.set_opacity(0.95)`) but reverted — dark terminal background blending against dark scrim produces no visible difference. See #572 for prior attempts and investigation notes.

**Breaks if:** Zooming any pane shows the overlay flush with the window edge (~5px gap) instead of a clearly distinct inset (~10px).

## 2026-05-03 — [CHANGED] Split DrawCommand into RenderCommand + HostCommand + ControlCommand (PR #621 → alpha)

`DrawCommand` split into three typed sub-enums: `RenderCommand` (paint primitives → `pending_frame`), `HostCommand` (side-effectful → `route_command`), `ControlCommand` (frame lifecycle + clipboard + logging, handled inline). The outer `DrawCommand` is now `#[serde(untagged)]` — wire format unchanged, existing apps require no updates. `route_command` takes `HostCommand` with an exhaustive match; adding a new variant without a handler is a compile error. The four manually-maintained dispatch lists (two in `mod.rs`, one in `routing.rs`, one in `render.rs`) collapse to a single exhaustive match per site. Also fixed a live bug: `AgentRosterGet` was missing from `background_tick()`'s dispatch list and was being silently dropped for background apps.

**Breaks if:** Any canvas app pane renders blank or shows routing errors in the log. Canvas apps that previously worked (todo, chat-poc, tool-poc) confirm routing is intact.

## 2026-05-03 — [CHANGED] Minimap at 75% opacity (PR #615 → alpha)

`ui.set_opacity(0.75)` added at the top of `render_minimap` in `src/minimap.rs`. Affects all painter calls (background, border, cells, labels) via egui's built-in opacity propagation — no per-color changes needed.

**Breaks if:** minimap appears fully opaque (no visible bleed-through of content behind it) after toggling on with ⌘⇧M.

## 2026-05-03 — [FIX] confirm_close default corrected to false (PR #614 → alpha)

`confirm_close()` used `unwrap_or(true)`, so users without a config file got pane-close confirmation dialogs on first launch. The generated config template always wrote `confirm_close = false`, creating a split: new-install behavior contradicted the documented default. Fixed by changing the fallback to `unwrap_or(false)`.

**Breaks if:** Cmd+W on a pane shows a confirmation dialog when `confirm_close` is absent from config.

## 2026-05-03 — [CHANGED] Kill Pane::Agent and Pane::AgentWorkspace — pane ADT is now Terminal | App (PR #612 → alpha)

Removed ~4,000 lines across 30 files. Deleted entirely: `agent_pane.rs`, `agent_workspace/`, `agent_workspace_modal.rs`, `render/agent_pane.rs`, `render/agent_workspace_pane.rs`, `process_app/agent.rs`. Removed from protocol: `AgentInit`, `AgentRoster`, `AgentInfo`, `AgentRosterGet`, `AppCommand::AgentRosterGet`. Removed from persistence: `SavedPaneKind::Agent/AgentWorkspace`, `SavedAgentWorkspace`. Removed from UI: `Action::OpenAgentPane`, `FocusLayer::AgentWorkspaceModal`, command palette agent workspace entries, `PaletteEntry::Action` infrastructure. Also removed: `AppRegistry::manifest_type()` / `system_prompt_for()` (only callers were in the deleted agent launch path), `process_app/mod.rs` roster test module. AI broker preserved — still processes `AiQuery`/`AiResponse` from PGAP apps. Agents are now regular apps that declare `ai.query` and own their turn loop (chat-poc proves the model).

**Breaks if:** terminal or app panes fail to spawn after merge, or `cargo test` drops below 258 passed.

## 2026-05-03 — [CHANGED] plexi update — detached relaunch when run from inside Plexi (PR #606 → alpha)

Added in-Plexi update flow to `self_update_cli()` in `src/cli.rs`. When `PLEXI_RUNNING=1`, the function downloads and stages the new bundle as usual, then writes a `nohup`-launched bash script to temp that polls `pgrep -x plexi`, `mv`s the staging bundle into place, re-symlinks the CLI, and opens the new app. It then triggers `osascript -e 'tell application "Plexi" to quit'` and exits 0. The external-terminal path (PLEXI_RUNNING unset) is unchanged. `nohup` chosen over bare spawn to survive SIGHUP when the Plexi terminal session closes on app quit.

**Breaks if:** `plexi update` from inside a Plexi pane prints "Restart Plexi to apply" instead of "Plexi will restart to apply the update." (indicating `PLEXI_RUNNING` isn't set in PTY env), or the app doesn't relaunch after quitting.

## 2026-05-03 — [CHANGED] plexi update — binary self-update for stable channel (PR #601 → alpha)

Implemented `self_update_cli()` in `src/cli.rs` (was a stub). Stable channel: fetches `/releases/latest` from GitHub API, compares versions, downloads `Plexi-<tag>.zip`, extracts via `unzip`, stages to `Plexi.app.update-staging`, atomically swaps old bundle, re-symlinks `/usr/local/bin/plexi`. Alpha/PR builds exit 1 with "update from source". Beta exits 1 with link to releases page (bundle rename requires install script). In-Plexi detached-relaunch flow deferred to #604.

**Breaks if:** `plexi update` on a stable build hangs or errors instead of printing "Already up to date" / downloading. Or: `plexi update apps` stops working (dispatches to the wrong handler).

## 2026-05-03 — [CHANGED] Reserve 'plexi update' for binary self-update; app updates → 'plexi update apps' (PR #600 → alpha)

`plexi update [<id>]` (app git-pull) renamed to `plexi update apps [<id>]`. Bare `plexi update` now routes to `self_update_cli()` — a stub that prints "not yet implemented" until #594 (binary download) lands. Dispatch change in `main.rs`; doc comments and error strings in `cli.rs` and `install.rs` updated. Breaking rename intentional on alpha before any scripts depend on the old surface.

**Breaks if:** `plexi update apps` fails to git-pull installed apps, or `plexi update` exits 0 instead of printing the not-implemented error.

## 2026-05-03 — [CHANGED] DrawCommand::SpawnPane — plexi open CLI, SDK ctx.spawn_pane(), panes.spawn capability (PR #598 → alpha)

Unified pane-spawn primitive: `DrawCommand::SpawnPane { type_id, layout, args, pipe_id }` replaces the narrow `SpawnApp` for new code. Adds `PlexiEvent::PaneSpawned`/`PaneSpawnError`, `Capability::PanesSpawn` ("panes.spawn"), `plexi open` CLI (file-based spawn-queue, same pattern as notify-queue), Python SDK `emit.spawn_pane()` + `on_pane_spawned`/`on_pane_spawn_error` hooks. `pipe_id` appended as `--pipe=<id>` to spawned app args for the AI↔human handoff loop. `layout: "background"` returns `PaneSpawnError` (blocked on #291). Overlay rendering (Z3 backdrop, Z2 anchor) deferred. Quick Note migration deferred. Also fixed pre-existing test harness struct literal errors (`show_cli_setup_prompt`, `file_picker_tx/rx` missing from `new_for_test`).

**Breaks if:** `plexi open <app-id>` silently does nothing (no new pane within ~2s), or an SDK app with `panes.spawn` calling `ctx.spawn_pane()` never receives `on_pane_spawned`.

## 2026-05-03 — [FIX] Cmd+Shift+A shows empty-state modal when notification queue is empty (PR #596 → alpha)

Three guards were fighting each other: (1) `ToggleNotificationModal` had an `else if visible_notification_count() > 0` that silently swallowed the action when empty. (2) `sync_notification_modal_focus` also checked count > 0, so Esc wouldn't close. (3) A frame-level `count == 0 → show_notification_modal = false` guard ran every frame and immediately killed the modal before the empty-state could render. All three removed. `draw_notification_modal` now handles the empty case by rendering the scrim + centered card with "No notifications" and Esc to close, instead of returning early.

**Breaks if:** Cmd+Shift+A with no pending notifications does nothing (no modal appears).

## 2026-05-03 — [FIX] notify --choice round-trip: response file never written (PR #586 → alpha)

`DeliverNotifyAction` has two dispatch paths: `dispatch_notify_action_cmds` (for early modal commands) and the main deferred handler. Response file write logic was only in the deferred path. Modal actions go through the early path — which also removes the notification from `pending_notifications` before dispatch fires. The main handler's lookup always returned `None` because the notification was already gone. Fix: added `response_file: Option<String>` to the `DeliverNotifyAction` variant so it travels with the command and is available on any path regardless of queue state.

**Breaks if:** `plexi notify --title "x" --choice "y:Yes" --timeout 10` blocks and then exits without printing the chosen key after clicking a choice button in the notification panel.

## 2026-05-03 — [FIX] False FREEZE alerts at startup from heartbeat spawning before shell probes (PR #589 → alpha)

`spawn_heartbeat` was called before `install_login_shell_path/env`, which spawn `zsh -i -l -c env` on the main thread. With `frame_tick` still 0 (eframe not started yet), the heartbeat correctly flagged 8-12s of no frames as a `[FREEZE]` — but incorrectly, since there was no UI thread to freeze. The false alerts contaminated every startup log and made the freeze appear correlated with the first eframe frames (including notify drain logs), blocking clean testing of PR #586's choice notification flow.

Fix: defer `spawn_heartbeat` to just before `eframe::run_native`. Shell probes complete on the main thread first; heartbeat only monitors actual eframe operation.

**Breaks if:** `[FREEZE]` lines appear in the log during the first 15s of idle launch with no user input.

## 2026-05-03 — [FIX] UI freeze + silent drop failure when dragging file onto zoomed pane (PR #585 → alpha)

Two root causes, both from misuse of egui pointer state during macOS OS-level file drags:

1. **Freeze (5-7s hang):** `TerminalView::show()` calls `backend.sync()` which clones the full terminal grid under `FairMutex` contention on every hover frame. On a full-window Claude Code session (zoomed = large grid), this blocked the main thread. Fix: during file hover on the zoomed overlay, skip `TerminalView` entirely and show a "Drop to paste path" indicator instead. The drop write happens before the render path so drop behavior is unchanged.

2. **Silent drop failure:** `dropped_to_zoom = has_drop && child_ui.rect_contains_pointer(inner_rect)` used egui's tracked pointer position, which is stale during macOS OS-level drags (the `NSApplication.mouseLocationOutsideOfEventStream()` probe is skipped when zoomed). The stale position only fell inside `inner_rect` by accident — when the pointer happened to be over the original split-pane location before zooming. Fix: when zoomed, unconditionally treat any drop as targeting the zoomed pane — there is no ambiguity since the entire overlay is one pane.

Also: cache `hovered_files` once per frame instead of O(n) per-pane `ui.input()` reads; move the zoomed early-return to the top of `pane_ui` to skip all input detection for background panes when zoomed.

**Breaks if:** dropping a file onto a zoomed terminal pane does not paste the path, or the freeze returns on drag hover.

## 2026-05-03 — [CHANGED] Show version in macOS menu bar for non-stable builds (PR #580 → alpha)

Appends the semver version to the bold app name in the macOS menu bar for alpha, beta, and PR builds. Stable shows "Plexi" unchanged. Derives the channel from `current_exe().file_stem()` (e.g. "plexi-pr-580" → "Plexi PR580 3.4.44") — the binary name is the canonical channel ID used throughout the codebase. Applied on the first `update()` frame via a one-shot `AtomicBool` because winit's activation delegate resets the menu item title after `PlexiApp::new()`.

Two wrong approaches tried first: (1) reading `app_menu_item.title()` — eframe/winit always sets this from `CARGO_PKG_NAME` regardless of `CFBundleName`; (2) calling `setTitle` in `new()` — winit resets it during activation. Both would have been caught by a single `log::info!()` before writing any code.

**Breaks if:** menu bar shows "Plexi" with no version on alpha or a PR build.

## 2026-05-03 — [FIX] Context naming modal when sidebar is hidden (PR #575 → alpha)

Creating a new context (⌘T) with the sidebar hidden left `renaming_window` set but the inline rename TextEdit never rendered — `suppress_focus` fired on every frame, locking the terminal permanently. Fix adds a `ContextRename` focus layer: when `renaming_window.is_some() && !sidebar_visible`, a centred modal (same UX as "Rename Pane") pops up. After Enter or Escape, terminal is immediately interactive. Sidebar-visible path unchanged.
**Breaks if:** Creating a new context with sidebar hidden shows no naming modal, or terminal is unresponsive after dismissing it.

## 2026-05-03 — [CHANGED] Drop-event breadcrumbs in zoomed overlay path (PR #581 → alpha)

Added `info`-level log lines around the drag-drop path in the zoomed overlay (`src/app/mod.rs`, `src/tiling.rs`). On drop into a zoomed pane the log now emits: overlay received (with `dropped_to_zoom` and `pane_id`), path written, and write completed. First use confirmed the freeze happens before any drop code fires — no `drop:` lines appeared despite the heartbeat catching a 2s+ UI-thread stall. Root cause is upstream of the drop path; see issue #582.
**Breaks if:** no `drop:` lines appear in the log after successfully dragging a file into any terminal pane (zoomed or not).

## 2026-05-03 — [CHANGED] First-launch CLI setup dialog + /usr/local/bin symlink (PR #566 → alpha)

On first GUI launch (drag-to-Applications path), a centered modal prompts the user to install the `plexi` CLI command. "Install" symlinks `/usr/local/bin/<cli-name>` → current binary and writes a sentinel (`config_dir/cli_setup_done`). "Not now" / Escape dismiss for the session only — dialog reappears next launch until installed. `just install` also creates the symlink (no dialog). Sentinel is channel-aware via `config_dir()` which reads the binary name from `current_exe()`.

Dropped the earlier `~/.local/bin` + `~/.zshrc` PATH-patching approach. `/usr/local/bin` is always on macOS PATH (Homebrew standard), user-writable without sudo, and requires zero shell config changes — same model as VS Code's `code` command.
**Breaks if:** CLI setup modal appears after clicking Install, or `/usr/local/bin/plexi` (or `-alpha`/`-pr-N`) symlink not created after clicking Install.

## 2026-05-03 — [FIX] Ghost empty state — welcome screen and Cmd+N guards + pr-install sed fix (PR #576 → alpha)

Closing a zoomed/fullscreen app pane could leave the window in a blank void: no terminals, no welcome screen, and Cmd+N misdirecting to a new page. Root cause: welcome-screen and Cmd+N guards both keyed on `panes.is_empty()` alone — when the tree had no root but panes retained stale entries, neither guard fired. Fix: both guards now also check `tree.root.is_none()`, making the empty void structurally impossible. `close_focused_app()` now clears `zoomed_pane` after `close_tile` to prevent stale tile references. Also fixed `scripts/install.sh` sed pattern (`"Plexi"` → `"Plexi[^\"]*"`) so `just pr-install` works correctly from the alpha channel (bundle name is `"Plexi Alpha"`, not `"Plexi"`).
**Breaks if:** welcome screen appears while a pane is open, or Cmd+N no longer creates new pages in a legitimate multi-page layout.

## 2026-05-03 — [CHANGED] ET timestamps on all changelog entries; bump-alpha now emits time (PR #574 → alpha)

Backfilled HH:MM ET onto every existing `## [X.Y.Z] — DATE` header in CHANGELOG.md using `git log -S "## [X.Y.Z]" --format="%ai" -- CHANGELOG.md` to find the actual commit time per version. Updated `bump-alpha` in justfile: `today` now uses `TZ=America/New_York date +%Y-%m-%d` and a new `time_et` var captures `HH:MM`; the section header is emitted as `## [X.Y.Z] — DATE HH:MM ET`. Some commits with `-0500` offsets (incorrect timezone on a machine) were normalized to EDT correctly by Python's `datetime.astimezone`.
**Breaks if:** `just bump-and-install` produces a new changelog entry with no time suffix (only `YYYY-MM-DD`).

## 2026-05-03 — [CHANGED] dismissable_modal helper — escape + click-outside for overlays (PR #570 → alpha)
Added `widgets::dismissable_modal(ctx, id, |ui| { ... }) -> bool` — handles Escape consumption and a click-absorbing scrim at `Order::Middle`. Shortcuts and changelog overlays both use it; the ✕ button in the changelog header was removed. Callers guard with `if !open { return; }` and apply `if dismissed { open = false; }` after. Any future centered info overlay should follow the same pattern.
**Breaks if:** Pressing Escape or clicking outside the shortcuts (⌘/) or changelog overlay does nothing.

## 2026-05-03 — [CHANGED] File picker capability — fs.pick / OpenFilePicker (PR #567 → alpha)
Added native file picker support: `DrawCommand::OpenFilePicker { request_id, filter, multiple }` triggers a native NSOpenPanel via `rfd::AsyncFileDialog` on a background thread (condvar-based `block_on` keeps the egui render loop unblocked). Result returns as `PlexiEvent::FilePicked` / `FilePickCancelled` through a dedicated `file_picker_rx` channel drained in both `background_tick()` and `ui()`. New `Capability::FsPick` (`"fs.pick"`) gates the call — denied apps get `FilePickCancelled` immediately. Python SDK gains `ctx.emit.open_file_picker()` + `on_file_picked` / `on_file_pick_cancelled` hooks. POC app ships under `examples/file-picker-poc/`.
**Breaks if:** Clicking the "Pick File" button in the file-picker-poc app opens no dialog, or a dialog opens but selecting a file never updates the displayed path.

## 2026-05-03 — [FIX] Terminal copy drops first character — iter_from excludes start point (PR #569 → alpha)
`selectable_content()` called `iter_from(range.start)` which excludes the start point, silently dropping the first character of every copy. Root cause: `open_link()` in the same file already demonstrates the correct pattern — explicitly index `grid[range.start]` before the loop. Fix: initialize `prev_line = Some(range.start.line)` and push the start cell before entering the `iter_from` loop.
**Breaks if:** Copying any text in a terminal pane still drops the first character (e.g. selecting `pass` pastes `ass`).

## 2026-05-03 — [CHANGED] Inject PLEXI_PANE_ID + PLEXI_SOCKET into every PTY (PR #565 → alpha)
Every PTY-backed terminal pane now receives `PLEXI_PANE_ID` (the numeric pane id) and `PLEXI_SOCKET` (variant-correct path to `~/.plexi-{channel}/notify.sock`) in its environment. Child processes inherit both vars naturally. `make_backend_settings()` gained a `pane_id: u64` first parameter; all six call sites updated. Socket path derived from `config::config_dir()` so it's automatically correct for alpha/beta/stable/PR builds. No new dependencies.
**Breaks if:** `echo $PLEXI_PANE_ID` in any terminal pane prints empty, or `echo $PLEXI_SOCKET` doesn't contain the channel-specific path.

## 2026-05-03 — [CHANGED] HostHarness: headless egui test harness (PR #564 → alpha)
`src/testing.rs` adds `HostHarness` for host self-validation without a real GPU or subprocess. `ProcessApp::new_for_test(pane_id, permissions)` takes explicit `AppPermissions` — pass `AppPermissions::builtin()` for normal tests, `from_capability_strings(&[])` for synchronous denial tests. AiQuery regression guard uses the denial path (no `ai.query` cap) so it's synchronous and deterministic — no sleep required. `background_tick()` drives routing directly when egui frames don't call `ui()` (zero-allocation headless panes). Pre-existing lint fixes: irrefutable `if let` patterns in `agent_pane.rs`, unused `use super::*` in two test modules.
**Breaks if:** `cargo test` reports fewer than 294 passed, or any of the 6 harness tests in `testing::tests` fail.

## 2026-05-03 — [CHANGED] Welcome screen: Plexi logo + centered wordmark (PR #562 → alpha)
Replaced the plain "PLEXI" text header with the Plexi 4-square logo alongside the PLEXI wordmark, centered as a unit. Logo is drawn via egui painter primitives (3 outlined squares + 1 purple-filled square, scaled from the icon.svg geometry) — no new dependencies. Centering uses the same manual-leading-pad pattern as the email row: measure text width via `ui.fonts()`, compute `pad = (available - total_w) / 2`, add it inside `horizontal()`.
**Breaks if:** welcome screen shows only the PLEXI text with no logo to its left, or the logo+wordmark group is left-aligned.

## 2026-05-03 — [CHANGED] Welcome screen email + clipboard (PR #560 → alpha)
Added contact section to the welcome screen: a dim caption message ("If you have any ideas, want to help, or just want to say what's up..."), ADHDISNTREAL@GMAIL.COM as a centered mailto: link, and an inline 📋 clipboard button that flips to ✓ for 2s after clicking. Centering is achieved by measuring the email text width via `ui.fonts()` before entering the horizontal layout, then adding explicit leading padding — egui's `vertical_centered` can't center a full-width `horizontal()` container directly. Also fixed `config_dir_name()` in `src/config.rs` to handle `pr-NNN` binary names so PR builds use their own isolated profile directory instead of falling through to stable.
**Breaks if:** email row is left-aligned on the welcome screen, or a PR build opens using `~/.plexi/` instead of `~/.plexi-pr-<n>/`.

## 2026-05-03 — [CHANGED] PR test-install flow: just pr-install / pr-clean (PR #559 → alpha)
Added `just pr-install <n>` and `just pr-clean <n>` to the justfile. `pr-install` compiles the current worktree and installs it as an isolated `Plexi PR<n>.app` with its own `~/.plexi-pr-<n>/` profile — lets you verify a feature in a real build before merging. `pr-clean` removes the app, binary, and profile dir after approval. Also fixed `install.sh` display name for `pr-NNN` channels: `pr-123` now produces `Plexi PR123` instead of `Plexi Pr-123`. Updated ship skill to run `pr-install` before the merge gate.
**Breaks if:** `just pr-install 123` from a feature worktree doesn't produce `/Applications/Plexi PR123.app`, or `just pr-clean 123` leaves artifacts behind.

## 2026-05-03 — [FIX] Sidebar hit-rect instability + renaming_window stale on reorder (PR #556 → alpha)
`SidebarRow::new()` was calling `ui.rect_contains_pointer()` at construction and baking the hover result into the content rect geometry — when hovered, the click/drag zone shrank by `ACTION_ZONE_WIDTH` (30px). This made the interaction rect change width frame-to-frame, causing unreliable hit areas especially on the second context item. Fix: geometry is now computed unconditionally from `action_enabled` only; the action glyph and `interact()` are gated on `hovered` inside `draw()`. Content rect is stable regardless of hover state. Also cleared `renaming_window` in all reorder paths (drag + menu MoveUp/Down/ToTop/ToBottom) to prevent a stale index applying a rename to the wrong context after reorder. Debug log added at `log::debug!` level for click-fallthrough-to-third-context investigation.
**Breaks if:** clicking a sidebar context item has no effect, or the × delete button appears on non-hovered rows.

## 2026-05-03 — [FIX] Tool registration pane_id race + token count error masking (PR #551 → alpha)
Two bugs found and fixed. (1) `ExposeTools` arrives before `set_pane_id`, so tools registered under pane_id=0 in the global `ToolRegistry`. On pane close, `unregister(real_id)` missed them — they leaked. Fix: routing now defers registration when pane_id is still 0; `set_pane_id` flushes exposed tools into the registry under the real id. (2) `fetch_generation_metrics` used `.ok()?` on the ureq HTTP call, silently swallowing all 4xx/5xx errors. Token counts were permanently 0/0 with zero diagnostics. Fix: proper error matching with debug-level logging of status codes, response bodies, and parse failures. Also increased retry delays (1s + 1.5s) for OpenRouter eventual consistency.
**Breaks if:** tool-poc tools don't appear in `ai_broker` log line when both tool-poc and chat-poc are open, or generation metrics fetch errors produce no log output at debug level.

## 2026-05-02 — [CHANGED] Markdown chat bubbles, TextInput scroll, counter shortcuts, pane focus (PR #549 → alpha)
Added `egui_commonmark` (v0.20, targeting egui 0.31) for host-side markdown rendering. New `DrawCommand::Markdown` + `ctx.markdown()` SDK method — ChatBubble now passes raw markdown to the host instead of line-by-line `ctx.text()`. Multiline TextEdit wrapped in `ScrollArea::vertical()` so it scrolls after filling its height instead of expanding below the screen. Counter tool-poc gained `on_key` handler (`i`/`+` increment, `r` reset). Pane keyboard navigation now auto-focuses the first TextInput via `pane_just_focused` flag.
**Breaks if:** chat-poc AI responses render as raw markdown text (no bold/italic), multiline input still expands below the screen instead of scrolling, counter keyboard shortcuts don't work, or navigating to chat pane via keyboard doesn't focus the text input.

## 2026-05-02 — [CHANGED] Parallax MVP editor + SDK ctx.image() (PR #548 → alpha)
Added `examples/parallax/` — primitive video editor driven by Parallax V0 CLI. Timeline with draggable clips, transport controls, I/O trim markers, ffmpeg export via linked terminal, 6 AI-callable tools (set_in_point, set_out_point, move_clip, trim_clip, list_clips, select_clip). Loads real clips from a project folder (ffprobe for duration, background thread) or runs in demo mode. Playback is render-driven (no sleep thread).

SDK: added `RenderContext.image(src, x, y, w, h, fit)` — emits `DrawCommand::Image` JSON. Host routing still logs "not yet implemented"; preview falls back to metadata display until the host renderer lands. The app auto-generates thumbnails via ffmpeg and will display them once Image is implemented.

Design choice: playback loop is render-driven (`schedule_render` from `on_render`) rather than a background thread with `time.sleep`. This avoids jitter and aligns with the host refresh cycle. Clip loading runs in a background thread to avoid blocking `on_init` with sequential ffprobe calls.
**Breaks if:** parallax app crashes on launch in demo mode, timeline clips don't render, or drag-to-reposition doesn't update clip start times.

## 2026-05-02 — [CHANGED] Chat bubble UI, TextInput refocus fix, review feedback (PR #544 → alpha)
Rewrote chat-poc with ChatBubble SDK component: user messages right-aligned (accent bg), assistant left-aligned (surface bg), error left-aligned (red bg). Added `multiline` param to TextInput component; chat now uses 64px multiline input with Shift+Enter for newlines. Removed model tier shortcuts (l/m/h) — hardcoded tier to "low".

Fixed TextInput permanently losing focus when pane is unfocused: the pane-level `Sense::click_and_drag()` interact widget was stealing pointer events from the TextEdit. Added explicit click-to-refocus that detects primary-button press inside TextInput rects.

Also addressed review feedback: `render_text_row` now uses `painter().galley()` with pre-measured galley instead of double-layout via `painter().text()`, default alignment changed to `LEFT_TOP` for consistency. `update_pane_context_snapshot` now skips the rebuild when total pane count hasn't changed.
**Breaks if:** chat-poc bubbles don't render (no colored backgrounds), TextInput still loses focus after clicking another pane and clicking back, or Shift+Enter doesn't insert newlines in the chat input.

## 2026-05-02 — [CHANGED] v3.7 context injection + TextInput fixes + list_view rename
Context injection (#396): AI broker now receives all open panes via a global snapshot updated each frame, not just the requesting pane. Implemented as a `PANE_SNAPSHOT` singleton in `broker.rs` written by `PlexiApp::update()` each frame, read by routing on `AiQuery` dispatch.

Text descender cutoff: `Label.measure()` and `Heading.measure()` in `ui.py` were returning exact baseline height with no room for descenders. Added `DESCENDER_PAD = 3.0` to both.

TextInput focus: host widget rect was hardcoded to 24px while SDK allocates 48px — clicks on the lower half missed the widget entirely. Now uses SDK-supplied `h` field on `DrawCommand::TextInput`. Also added `text_input_has_focus` flag so `handle_key` suppresses app key forwarding while typing (typing "h" in chat input no longer triggers tier change).

Renamed `ctx.list()` → `ctx.list_view()` to fix Pyright error from shadowing Python's `list` builtin.
**Breaks if:** chat-poc text input is unclickable after AI responds, text descenders still clipped, or example apps using `ctx.list_view()` crash with AttributeError.

## 2026-05-02 — [CHANGED] Add text_row() host-measured layout primitive (PR #540 → alpha)

Added `text_row()` method to RenderContext for rendering multiple text segments horizontally with SDK-owned spacing and alignment. Host measures each segment with real font metrics and flows them left-to-right; eliminates manual position math and makes padding mishaps impossible. Configurable via items (dict with text/color/size/monospace), gap spacing (default 8px), and alignment. Updated input-inspector to use it instead of hardcoded `div_x + PAD + 64` offsets, resolving the visual gap between timestamp and event in the log.
**Breaks if:** text_row is called without a "text" key in an item dict (raises ValueError), or rendered log entries show visual gaps/overlap in input-inspector.

## 2026-05-02 — [CHANGED] Add input-inspector POC app (PR #529 → alpha)

Added `examples/input-inspector/` — a two-panel PGAP app that exercises every input event type (key, click, mouse movement, scroll). Left panel is interactive with live cursor position and tracking dot; right panel is an event log colour-coded by category. Verifies all input events deliver correctly after issue #331 investigation.
**Breaks if:** mouse movement or scroll events don't appear in the event log of input-inspector.

## 2026-05-02 — [FIX] AiQuery silently dropped — never reached route_command (PR #536 → alpha)

`DrawCommand::AiQuery`, `ExposeTools`, `ToolResult` (and `OpenVideo`, `CloseVideo`, `SetVideoState`, `Image`, `AudioMeter`) were absent from the dispatch match lists in both `ui()` and `background_tick` in `process_app/mod.rs`. They fell through to `other => pending_frame.push(other)` / `_ => {}` — silently dropped, never routed. Root cause of chat never working since PR #526 introduced `AiQuery`. Any new `DrawCommand` variant that has a `route_command` handler must also be added to both match lists or it will be silently discarded.
**Breaks if:** chat-poc sends a message and gets no response (35s timeout in log).

## 2026-05-02 — [GOTCHA] bump-alpha SIGPIPE with pipefail — use `git log -1` not `| head -1`

`git log --grep=... --format="%H" | head -1` causes git to receive SIGPIPE when `head` closes after the first line. With `set -euo pipefail`, this fails the whole script. Fix: pass `-1` directly to `git log` so it emits one line and exits cleanly — no pipe truncation needed.

## 2026-05-02 — [FIX] Three root causes behind chat-poc failures (PR #534 → alpha)

**Three bugs found and fixed:**

1. **TextInput loses focus after first Enter (host-side).** egui's singleline TextEdit drops focus on Enter. Auto-focus only fires for `newly_visible` inputs (first frame). After the first submit, focus was never restored — user had to click the input to type again. Fix: `resp.request_focus()` after singleline submit detection.

2. **AiResponse delayed by one frame (outbound flush timing).** `outbound_events` (including broker `AiResponse`) were flushed only at the START of the next `ui()` call. Events drained from `http_rx` during a frame sat in the buffer for one full frame cycle (~100ms with idle polling). Combined with the SDK's 35s async timeout, this created a delivery race. Fix: added `flush_outbound_events()` at the END of `ui()` so events are delivered same-frame.

3. **Env probe logged count but not key names.** "Adopted 5 env vars" gave no way to verify OPENROUTER_API_KEY was among them. Fix: log line now prints `Adopted N env vars from login shell: [KEY1, KEY2, ...]`.

**PGAP integration test confirmed** the SDK-level flow works correctly — two consecutive text_submitted → ai_query round-trips succeed. The follow-up text input bug was purely host-side (egui focus), not a protocol issue. The `-i -l` env probe fix (PR #533) was verified to produce OPENROUTER_API_KEY even from `</dev/null` (no tty). The installed binary was stale (PR #532's parallel install overwrote #533's build).

**Breaks if:** text input loses focus after pressing Enter in chat-poc (regression). Or: `~/.plexi-alpha/plexi.log` shows "Adopted N env vars" without key names. Or: AI response still times out at 35s on a fresh 3.4.12 launch.

## 2026-05-02 — [GOTCHA] OPENROUTER_API_KEY env-adoption — multiple failed attempts, still broken

**Symptom (still active as of 04:49:55):** chat-poc emits `ai_query timed out after 35s — check OPENROUTER_API_KEY and network connectivity` on every send. `~/.plexi-alpha/plexi.log` shows `Adopted 5 env vars from login shell` on every launch — same value before and after each "fix". Five is suspicious: the user's login shell exports dozens of vars, and `OPENROUTER_API_KEY` is reachable manually (`zsh -i -l -c 'echo $OPENROUTER_API_KEY'` returns the key).

**What was tried — all failed:**

1. **PR #531 — added `install_login_shell_env()` running `zsh -l -c env`.** Hypothesis: GUI bundles only inherit minimal env, so probe the login shell and `set_var` anything missing. Shipped to 3.4.10, installed, log showed "Adopted 5 env vars". Did not fix the timeout. Did not verify whether `OPENROUTER_API_KEY` was one of the 5. **Mistake: shipped without checking which vars were adopted — "5" is the same answer as broken.**

2. **PR #533 — changed `-l -c env` to `-i -l -c env`.** Hypothesis: `~/.zshrc` (which sources `~/.zsh_secrets`) only loads in interactive shells, so `-l` alone misses it. Verified the hypothesis with a clean-env CLI test (`env -i /bin/zsh -l -c env | grep OPENROUTER` → empty; `-i -l` → key present). Shipped to 3.4.11. Log STILL shows "Adopted 5 env vars". **Either the 3.4.11 binary does not actually contain `1bf4eae` (the parallel session that ran `just install` for PR #532's bump may have built from a worktree that pre-dated my merge), or the probe is failing for a different reason and `-i -l` was never the bottleneck.**

3. **Did not consider:** running `just install` *myself* from `worktrees/alpha/` after my fix merged, to guarantee the installed binary contains my commit. I trusted the parallel session's install. The "5 vars" count is direct evidence the running binary is not the one I thought I shipped.

**What I never investigated and should have:**

- Which 5 vars are being adopted? Add a one-line `log::info!` with the adopted keys (or at least the count + a sample) before declaring the probe "working".
- Is `1bf4eae` actually in the running binary? Add a build-stamp log line tied to git SHA — every `Plexi Alpha.app` should print its commit on startup, so we can rule out stale binaries in 1 second.
- Does `Command::new(&shell).args(["-i", "-l", "-c", "env"])` behave the same way when invoked from a GUI process (no controlling tty) as it does from a terminal? Interactive zsh may detect no-tty and skip `.zshrc` even with `-i`. **This is the most likely real root cause and was never tested.**
- The existing `crate::secrets` module (Keychain-backed, issue #296 has the live ticket in v3.9) is a working alternative path that bypasses the shell-probe problem entirely.

**Honest assessment of failure mode:** I treated a probe-output count of "5" as success because the env-adoption code ran without error, then shipped twice without verifying behavior end-to-end. The user's 35s timeout is exactly the same before PR #531, after PR #531, and after PR #533. Two ship cycles burned. The next agent should not assume `-i -l` works in a GUI subprocess context — verify by logging the adopted keys *and* the number, and ideally check whether `Command::new` even sees `~/.zshrc` without a tty.

**Breaks if:** literally still broken — chat-poc times out at 35s. To verify a real fix, look for `OPENROUTER_API_KEY` in the adopted-keys log line *and* see chat-poc reply within ~5s.

## 2026-05-02 — [FIX] Use `-i -l` for env probe so .zshrc-defined secrets load (PR #533 → alpha)

PR #531 added `install_login_shell_env()` to adopt user secrets from the login shell, but used `zsh -l -c env` (login-only). Login mode loads `~/.zprofile` / `~/.zlogin` but NOT `~/.zshrc` — the shell is non-interactive. Secrets sourced from `.zshrc` (e.g. `~/.zsh_secrets` containing `OPENROUTER_API_KEY`) were therefore invisible to the GUI bundle, and chat-poc would still hit "ai_query timed out after 35s — check OPENROUTER_API_KEY" on a fresh launch.

**Fix (DID NOT WORK — see GOTCHA entry above):** `zsh -l -c env` → `zsh -i -l -c env`. Interactive mode forces `.zshrc` to load alongside the login profiles. Verified locally: `env -i /bin/zsh -l -c env | grep OPENROUTER` returned empty; same probe with `-i -l` returns the key. Shipped to 3.4.11; behaviour unchanged (still "Adopted 5 env vars"; chat-poc still times out at 35s).

**What NOT to do:** Don't pivot to a Plexi-managed secrets vault for this. The vault is real (issue #296, v3.9 milestone — `crate::secrets` Keychain-backed scaffolding already exists), but shell-env adoption is the standard macOS GUI bundle fix used by every comparable tool (iTerm, Warp, VS Code, Cursor). Users will always have keys in `.zshrc`; we shouldn't force them into a Plexi UI just to make Plexi work. The two systems coexist — vault for first-class Plexi-only secrets, shell-env adoption for existing user setups.

**Breaks if:** chat-poc still times out at 35s on a fresh GUI launch despite `OPENROUTER_API_KEY` being set in `~/.zshrc`. Or: log shows fewer than ~10 vars in "Adopted N env vars from login shell" line for a typical user shell. **(Both currently broken — see GOTCHA above.)**

## 2026-05-02 — [FIX] Tighten freeze watchdog + throttle macOS drag-cursor polling (PR #532 → alpha, closes #396? no — closes #530)

Four silent crashes on alpha during a single ~35-min window today (sessions starting 03:27, 03:53, 03:56, all dying without panic or `.ips`). Triggered by dragging an image into a zoomed Claude Code terminal pane — macOS spinning ball, then SIGKILL. Watchdog (added earlier) was running but never logged `[FREEZE]`: the 3453ms peak stall fell under the 5s threshold.

**Three changes:**
1. `logging.rs`: `SAMPLE_INTERVAL` 5s → 1s, `FREEZE_THRESHOLD_SECS` 5s → 1s, `HEARTBEAT_EVERY_N_SAMPLES` 6 → 30 (heartbeat cadence preserved). The next freeze will leave a `[FREEZE]` line in the log before the OS kills the process.
2. `app/mod.rs`: drag-cursor `request_repaint_after` 16ms → 100ms. Calling `NSApplication.mainWindow` + `mouseLocationOutsideOfEventStream` 60×/sec from the render loop is unjustified regardless of whether it's the root cause.
3. `app/mod.rs`: skip the entire unsafe ObjC cursor probe when a pane is zoomed. Targeting is moot when one pane fills the window — this removes the suspect code path from the exact #530 scenario.

**What this is NOT:** root-cause fix. Could not prove the ObjC calls cause the freeze; the 3.4s stall could equally be `TerminalView` rendering a large pty output burst. SIGKILL bypasses every Rust handler, so the watchdog can only buy diagnostics, not survival. True per-pane crash isolation requires architectural work not yet started.

**Breaks if:** No `[FREEZE]` lines appear in `~/.plexi-alpha/plexi.log` after a freeze recurs (sample interval not firing). Or: heartbeat cadence drifts off 30s. Or: dragging a file into a non-zoomed pane no longer focuses the correct pane (cursor probe broke after throttle).

## 2026-05-02 — [FIX] Login-shell env inheritance + TextInput layout widget (PR #531 → alpha)

**Root cause 1 — API keys invisible to Plexi:** `install_login_shell_path()` only adopted `PATH` from the login shell. `OPENROUTER_API_KEY` and other user secrets set in `~/.zsh_secrets` (sourced from `.zshrc`) were never in the process env. Added `install_login_shell_env()`: runs `zsh -l -c env`, skips system/terminal vars (HOME, USER, TERM, etc.), sets any remaining var not already in the process env. Called in `main()` immediately after `install_login_shell_path()`. Log line "Adopted N env vars from login shell" confirms it ran.

**Root cause 2 — TextInput overlap with Column children:** `ctx.text_input()` requires manual absolute coordinates. Apps that used it alongside a `Column` (which renders `Divider`/`Footer` at the bottom) had the input float at the absolute position, visually overlapping the Column's bottom children. Added `TextInput` to `sdk/python/plexi_sdk/ui.py` — a proper `Component` subclass that participates in Column layout: `measure()` returns fixed height (48px default), `render()` calls `ctx.text_input()` with the layout-computed `(x, y, w)`, and `.submitted` property exposes the result after `ctx.render()`. `chat.py` migrated to use it.

**What NOT to do:** Never use `ctx.text_input()` with manual absolute coords inside an app that also uses a Column — they have no shared coordinate authority. Always use `TextInput` as a Column child instead.

**Breaks if:** chat-poc text input box overlaps the footer (regression). Or: `from plexi_sdk.ui import TextInput` raises ImportError. Or: log does NOT show "Adopted N env vars from login shell" on alpha startup (env probe failed silently).

## 2026-05-02 — [CHANGED] v3.7 complete — app tool protocol + host context injection (PR #526 → alpha)

Full v3.7 milestone: ExposeTools/ToolCall/ToolResult PGAP protocol, global tool registry (`src/plexi_ai/tool_dispatch.rs`), multi-round broker tool loop, OpenRouter streaming tool_call delta accumulation, host context injection, Python `@app.tool` decorator, and `examples/tool-poc/` counter POC.

**Key decisions:** Tool registry is global (not window-scoped) for MVP — last-write-wins on name collision, log warning. Window scoping is v4 cleanup. Messages type widened to `Vec<serde_json::Value>` inside the backend so the broker can inject assistant tool_call and tool-result turns without changing the PGAP wire format. Tool loop capped at 10 rounds (log warn if hit). `# no-host-context` in system prompt opts out of context injection.

Gemini catch: `int(args.get("n", 1))` → `int(args.get("n") or 1)` in counter.py — `args.get("n", 1)` returns `None` when LLM passes `"n": null` (key exists, value is null), so the default doesn't fire and `int(None)` raises.

**Breaks if:** `from plexi_sdk import App` raises ImportError. Or: chat-poc AI response arrives but counter pane count doesn't change after "increment 3 times". Or: context prefix missing from broker log when `# no-host-context` is NOT in system prompt.

## 2026-05-02 — [CHANGED] Changelog modal + just bump-alpha (PR #524 → alpha)

Version badge in toolbar (`v3.x.x`) is now a clickable button that opens a scrollable changelog modal. `CHANGELOG.md` is embedded at compile time via `include_str!` — no runtime file access. Escape or ✕ closes it. `show_changelog: bool` follows the same plain-bool pattern as `show_shortcuts` (not FocusLayer — Gemini suggested it but `show_shortcuts` ships fine without it).

`just bump-alpha` added: patch-only bump, no release build, no tag. Pulls commits since last tag via `git log`, strips `chore: DEV_LOG` and bump commits, prepends an `## [alpha]` block to `CHANGELOG.md`, commits. After `just install` the modal shows alpha work at the top.

Gemini fixes applied: `mapfile` → `while read` (Bash 3.2 compat), `\n` strings → `$'\n'` for real newlines, `awk` now matches first `## ` header to insert before (not the `# Changelog` title), `**` bold markers stripped before rendering.

**Breaks if:** clicking `v3.x.x` in toolbar has no effect, or changelog modal doesn't open.

## 2026-05-02 — [CHANGED] v3.8 partial — ListItem/Row components + error visibility (PRs #521, #522 → alpha)

**#521 (#388):** Added `ListItem` and `Row` components to `sdk/python/plexi_sdk/ui.py`. Both handle vertical centering internally — eliminates the `ctx.text` `align=` omission bug and manual y-offset math (`h * 0.38`, `h * 0.72`) that was producing subtle layout errors in descriptor-renderer. `ListItem` is a single/double-line item card (title, subtitle, leading icon, trailing); `Row` is a horizontal info row (leading icon, label, trailing badge/chevron). Gemini review: used allocated `h` for subtitle positioning instead of `self._h()` for consistency. Declined padding-on-Row suggestion — `Row` has no background rect; inner padding is the container's concern.

**#522 (#424 partial):** Two process_app error visibility fixes that don't require v3.6 AI plumbing:
- `lifecycle.rs`: `BOOT_TIMEOUT=10s` — apps that never send `Ready` now flip to `Crashed` after 10s instead of hanging on "starting…" indefinitely.
- SDK: consecutive render exception counter (threshold=3) — after 3 consecutive `on_render` failures, `traceback.print_exc()` + re-raise. Traceback hits stderr before `os._exit(0)` terminates the process; lifecycle flips to `Crashed` via stdout EOF regardless, but diagnostic info now appears in the host log. Counter resets on any successful render.
Remaining #424 item (AI config error surfacing in-pane) deferred until v3.6 AI plumbing is stable.

**Breaks if:** `from plexi_sdk.ui import ListItem, Row` raises ImportError. Or: an app frozen in `Booting` for >10s doesn't flip to "crashed" pill. Or: an app whose `on_render` always throws shows no "crashed" indicator after 3 frames.

## 2026-05-02 — [CHANGED] v3.6 complete — chat-poc + docs/specs deletion (PRs #518, #519 → alpha)

**#519 (#508):** Added `examples/chat-poc/` — conversational chat POC proving AiQuery/AiResponse round-trip via OpenRouter end-to-end. Tier selector (l/m/h), full multi-turn history, loading state, inline error display, auto-scroll to bottom on new turns. Replaced `ai-query-test` as the canonical AI demo. Gemini review caught a real render-order bug (text_input must come after ctx.render() — render clears the pane first), private member access for scroll-to-bottom (fixed by setting scroll_offset=1_000_000 and letting Scrollable.render() clamp it), and double-padding on the inner Column (fixed with padding=0).

**#518 (#509):** Deleted entire `docs/specs/` directory — all release specs, subsystem specs, proposals, and process docs were out of date. Removed all dangling references from CLAUDE.md, AGENTS.md, ARCHITECTURE.md, .claude/iteration-cycle.md, examples/plexi-descriptor-demo/README.md, and three Rust source comments.

**Breaks if:** chat-poc crashes on launch, or `grep -r "docs/specs" .` (outside DEV_LOG/SPRINT) returns results.

## 2026-05-02 — [DECISION] v3.8 partial start — #388 and #424 independent work split off

v3.8 has 4 issues. #394 (streaming) and #395 (in-pane agent) depend on v3.6's AiQuery plumbing and are deferred until that lands. #388 (SDK layout components) and part of #424 (error visibility) are independent and open as PRs now.

PRs open: #521 (#388 ListItem/Row), #522 (#424 partial). Remaining v3.8 work (#394, #395, AI config error surfacing from #424) waits on v3.6.

## 2026-05-02 — [CHANGED] WorkspaceRouter — compile-enforced context switching (PR #510 → alpha)

Extracted `active_context` and `contexts` into `src/workspace_router.rs` with private fields. Direct `active_context = n` is now a compile error outside that module. `PlexiApp::switch_workspace` remains the only navigation path (saves/restores minimap). Structural ops (create/delete/reorder) are atomic router methods that maintain active-index coherence internally. `remove_at` adjusts active automatically; `debug_assert!` guards `new` and `set_active`. Closes #380.

**Breaks if:** Context switching via sidebar, ⌘1-9, or command palette doesn't save/restore minimap visibility per context.

## 2026-05-02 — [CHANGED] v3.5 batch — plexi_iq deletion, config migration, egui-term view fixes (PRs #502–504 → alpha)

Three parallel sub-agent PRs:

- **#502 (#429):** Deleted `src/plexi_iq/` — 1,238 lines of dead code. All IQ→AI rename work was already done; the directory was just orphaned (no `mod plexi_iq` in `main.rs`).
- **#503 (#425):** Added `scripts/migrate-config.sh` + wired into `scripts/install.sh`. Checks for missing top-level sections (`[ai]`, `[notifications]`, `[theme]`, `[beta]`) and appends them additive-only. Tested live: caught missing `[notifications]` and `[ai]` on an existing install.
- **#504 (#475, #492, #472):** `view.rs` fixes — empty clipboard guard (`trim().is_empty()` before WriteToClipboard), auto-scroll boundary fixed (triggers on pane exit, not 20px inside), scroll speed proportional to overshoot. `cell_height.max(1.0)` guard added to prevent divide-by-zero at tiny font sizes. #472 (HiDPI column mismatch) investigated — no mismatch found in current code; both SIGWINCH cols and renderer cell_width floor the same value identically.

**Breaks if:** `cargo build` references `plexi_iq` module. Or: `just install` on a config missing a section doesn't add it. Or: drag-selecting an empty terminal region overwrites clipboard.

## 2026-05-02 — [DECISION] #380 WorkspaceRouter deferred to v3.6

The actual bug (SwitchContext bypassing `switch_workspace`) was already fixed. No direct `self.active_workspace =` assignments exist outside `switch_workspace`. The issue is a structural refactor to make the bug class a compile error — valid engineering but no active risk. Cost is high (touches 6+ files), benefit is insurance only. Deferred.

## 2026-05-01 — [CHANGED] commit-graph batch — flat load, host scroll, badge fixes, PR badges (PR #500 → alpha)

Four issues landed as one PR:
- **#488:** Replaced week-window navigation (`[`/`]`/`t`) with flat `max_count=100` load. `fetch_commits` and `fetch_numstats` now take `max_count` instead of time bounds. `_week_offset` state removed entirely.
- **#487:** Migrated `push_clip`/`pop_clip` → `begin_scroll`/`end_scroll`. Added `on_scroll` handler — host sends `offset_y` on trackpad/wheel events, app stores it and schedules render. j/k still updates `_graph_scroll_offset` directly; it's reconciled on the next `begin_scroll` call.
- **#489:** Badge overlap fixed by computing `item_w` per-item in `_draw_legend` instead of a single `max_item_w`. Legend capped at 2 rows with `+N more` overflow.
- **#490:** `_parse_commits` now extracts `pr_number` via `re.search(r'\s\(#(\d+)\)$', subject)`. Rendered as a muted `#NNN` badge after ref badges in the node loop.

**Breaks if:** Trackpad scroll doesn't move the commit list (begin_scroll not wired). Or: `[`/`]` keys still appear in footer/help (cleanup incomplete). Or: Legend items overlap on multi-branch repos (per-item width regression).

## 2026-05-01 — [CHANGED] sidebar_row.rs — zone-based row abstraction (issues #480, #481, #483)

Introduced `src/sidebar_row.rs` with `SidebarRow` / `RowLayout` / `RowResult`. The abstraction enforces:
- **Row rect from cursor origin, not allocation response.** `SidebarRow::new()` captures `ui.cursor().min` before any allocation. The old code used `allocate_ui_with_layout.response.rect` which returns the bounding box of rendered content — a short name like "ff" gave a 70px-wide row rect, placing the X zone at x=40 instead of x=190.
- **Zones fixed before any rendering.** `RowLayout` derives `full`, `content`, and `action` rects at construction. Content layout cannot retroactively shift zone boundaries.
- **Single cursor authority.** `RowLayout::resolve_cursor()` is the only place the cursor is set. It checks `rect_contains_pointer` on the zone rects — zone-based, not pixel-based widget response.
- **No `interact()` inside layout closures.** All interaction registration happens after content rendering, in a predictable order with no overlapping rects.
- **Typed `RowResult`.** Callers read `result.primary_clicked`, `result.action_clicked`, etc. — no scattered state mutations inside the render path.

**What NOT to do:** Don't compute the row rect from `allocate_ui_with_layout.response.rect` — it's the content bounding box, not the requested size. Always snapshot `ui.cursor().min` before allocating.

**Breaks if:** X button zone appears at wrong horizontal position (cursor origin not captured before allocation). Or: Grab cursor shows over the X zone (cursor set after the zone check). Or: clicking anywhere on a short-name row registers the X button (rects overlap).

## 2026-05-01 — [FIX] Sidebar cursor/click bugs — three root causes (issues #480, #481, #483)

**Bug #483 (separator line on hover):** `SidePanel::left()` is resizable by default — egui renders a drag handle on the right edge. Fixed by adding `.resizable(false)`.

**Bug #481 (I-beam cursor on context names):** In egui 0.31, `Label` shows I-beam for selectable text regardless of `.sense()`. Added `.selectable(false)` to suppress it. Also fixed the cursor override path below.

**Bug #480 (X button unclickable) + cursor inconsistency:** Two compounding causes:
1. The early `ui.interact(ui.max_rect(), id, Sense::click_and_drag())` inside `allocate_ui_with_layout` claimed the entire row rect before the X button was registered, making event ownership ambiguous.
2. `ui.ctx().set_cursor_icon(Grab)` ran *after* the entire closure, unconditionally overriding the X button's `on_hover_cursor(PointingHand)` — users saw Grab everywhere and naturally didn't click.

Fix: removed the early `interact()` from inside the closure. Hover detection now uses `ui.rect_contains_pointer()` (pure query, no event registration). Drag interaction is a separate `ui.interact()` on a sub-rect that geometrically excludes the X button zone (`row_rect` minus last 30px when X is visible). Cursor set once after all rendering with correct priority: X button hovered → PointingHand, else row hovered → Grab.

**What NOT to do:** Don't put `ui.interact(max_rect, ...)` inside `allocate_ui_with_layout` when the layout will also contain interactive widgets — the outer interact claims the whole area and competes with inner widgets for both events and cursor icon.

**Breaks if:** X button shows Grab cursor instead of PointingHand when hovering it. Or: I-beam appears on context name labels. Or: sidebar has a visible resize handle/separator line on the right edge.

## 2026-05-01 — [CHANGED] `just install` now channel-aware — works from any worktree

Updated both `CLAUDE.md` (root) and `worktrees/alpha/CLAUDE.md` to reflect that `just install` reads `.channel` from CWD and dispatches to `install-alpha`, `install-beta`, or `install-stable` automatically. Removed all guidance that said "use `install-alpha` for alpha, `install` for main" — that distinction no longer exists. The canonical install command is now `just install` from whichever worktree you're in.

## 2026-05-01 — [CHANGED] Remove PLEXI logo + toolbar label; normalize dot sizes

Removed the "PLEXI" heading and divider from the sidebar top (`sidebar.rs`). Removed the context/pane title label from the toolbar (`overlays.rs` `draw_toolbar`). Toolbar dots and tab dots both set to radius 4.0, spacing 12.0 — a slight bump from the original 3.5/4.0 without oversizing.

## 2026-05-01 — [CHANGED] Shortcuts overlay redesign — two-column layout, square chips, HJKL blocks

Widened the shortcuts overlay from 320px to 620px and split it into two columns. Left: pane/window management. Right: navigation (HJKL blocks at top). Key chips are now square for single-char keys (`chip_w` floor raised from fixed 18px to `chip_h`). HJKL navigation rendered as `⌘ [H][J][K][L]` and `⌘⇧ [H][J][K][L]` blocks instead of paired two-combo rows. Fixed stale label: `⌘N` → "New terminal", `⌘⇧N` → "New context".

## 2026-05-01 — [FIX] Reload Configuration keyboard shortcut not firing

`Cmd+Shift+,` via egui's `consume_key` was unreliable — macOS can consume shift+comma before egui sees it. The "Reload Configuration" menu item also had an empty `keyEquivalent`, so macOS had no knowledge of the shortcut. Fixed by setting `keyEquivalent: ","` and `keyEquivalentModifierMask: NSEventModifierFlagCommand | NSEventModifierFlagShift` on the menu item in `macos_menu.rs`. The macOS menu now intercepts the key and fires the `RELOAD_CONFIG_FLAG` atomic → `reload_config()` path, which is the same path that clicking the menu item uses and is known to work.

## 2026-05-01 — [CHANGED] Gut TextEditorApp — Cmd+, now opens system editor

Deleted `src/text_editor_app.rs` entirely. `Action::OpenConfig` (Cmd+,) now calls `open_config_file()` — the same `open` invocation the macOS menu uses — instead of spawning the half-baked in-app text viewer. `Cmd+Shift+,` (`ReloadConfig`) was already correct and unchanged.

## 2026-05-01 — [CHANGED] Command palette — context/pane model overhaul

Rewrote the palette entry logic three times to converge on the right model. Key decisions:

**Terminology settled:** Context = sidebar project item (`Context.name`). Window = spatial grid page within a context (`Window.name` — never user-set, always auto-generated). Pane = individual terminal/app split (`TerminalPane.name: Option<String>` — user-set via rename-pane overlay). The palette should surface contexts and named panes; windows are invisible to the user.

**Window.name stripped from palette entirely.** Old auto-names ("Page X,Y", "Context N") were written before windows defaulted to `""`. Migration strips them on load via `is_auto_window_name()` in `app/mod.rs`. New windows get `name: String::new()` at creation.

**Primary entry model:** Each context gets exactly one primary entry. If the active pane (context_active_window → focused_pane → TerminalPane.name) is named, that named pane IS the primary entry ("context › pane-name"). If unnamed, the context name is shown bare. Additional named panes (not the active one) appear as secondary entries below. This preserves the ⌘P → ↓ → Enter flow — the first entry for each context always represents wherever you last were.

**Named pane jump now focuses the pane.** `jump_to_context` gained an optional `pane_id` param. When set, it finds the `TileId` for that pane in the window's tile tree and sets `window.focused_pane`.

**`delete_window` cleanup:** Added `context_active_window` update when the deleted window was the stored last-visited for its context, preventing palette navigation to ghost window IDs.

**Minimap page numbers:** Changed from 1-based to 0-based (`p + 1` → `p` in minimap.rs).

**Breaks if:** Jumping to a named pane from the palette doesn't focus that pane (tile tree walk fails). Or: a context with a named active pane shows a bare context-name entry instead of the pane name (primary entry logic fell through to unnamed path).

## 2026-05-01 — [CHANGED] Gut pulse beta feature — applied to full window border, not per-pane

Removed `pulse` entirely: `BetaConfig.pulse` field, the `# pulse = false` line in the config template, the `features.rs` flag insertion, and the `draw_feature_effects` rendering block. The implementation painted a breathing glow rect over `ctx.screen_rect()` — the entire window border — with no per-pane awareness. Gutted rather than fixed; can be revisited properly when the renderer has a focused-pane rect to target. `ghost` and `crt` flags are unaffected.

## 2026-05-01 — [GOTCHA] theme_preset silently ignored — TOML section ordering

`theme_preset` is a top-level field in `PlexiConfig` but `CONFIG_TEMPLATE` placed it after the `[notifications]` section header. TOML parses all keys after a `[section]` header as belonging to that section, so `theme_preset` was deserialized as `notifications.theme_preset`. `NotificationsConfig` has no such field, serde silently drops it, and `PlexiConfig.theme_preset` is always `None` — no preset ever applied.

Fix: moved `theme_preset` above `[notifications]` in `CONFIG_TEMPLATE`. Also updated `~/.plexi-alpha/config.toml` (the existing user config had the same ordering).

**What NOT to do:** Any top-level TOML key added to `PlexiConfig` must appear in the template *before* the first `[section]` header, or it will silently be swallowed by that section. Never place bare keys between two section headers in the template.

## 2026-04-30 — [FIX] Async input handlers stall event loop; import crash from list builtin shadow (PR #466 → alpha)

**Issue #393 — blocking in event handlers freezes frame loop.** `_dispatcher` used `await _dispatch_hook(hook, ...)` for all events including input hooks. A slow `on_key` suspended the dispatcher — no further events processed, app froze. Fix: `_dispatch_hook_task` schedules async hooks via `asyncio.create_task()`. `on_render`, `on_init`, `on_shutdown` still awaited for ordering. Task refs stored in `_background_tasks` set to prevent GC; `_log_task_exception` callback surfaces unhandled errors.

**Pre-existing import crash from PR #460.** `RenderContext.list` shadowed the builtin `list` in the class body, causing `list | None` annotations on later methods to fail at class definition time on Python 3.12. Fixed by quoting the three affected annotations.

**Breaks if:** An async `on_key` that awaits slow I/O freezes the frame loop. Or: `import plexi_sdk` crashes on Python 3.12.

## 2026-04-30 — [FIX] TextInput: auto-focus only first node + Shift+Enter multiline (#404 → alpha)

Two bugs in `render_text_inputs` in `src/process_app/mod.rs`.

**Bug 1 — auto-focus last node wins instead of first:** `request_focus()` was called on every newly-visible TextInput in a frame. egui's `request_focus` is last-write-wins within a frame, so when multiple inputs appear together (e.g. a form), only the last one received focus. Fixed by tracking a `focus_granted` bool per call and only requesting focus on the first newly-visible input.

**Bug 2 — Shift+Enter did nothing in multiline mode:** egui's multiline `TextEdit` only inserts `\n` for plain Enter — it does not handle Shift+Enter at all. The existing code intercepted plain Enter to submit (stripping the egui-inserted `\n`), but the Shift+Enter case fell through to egui with no result. Fixed by explicitly detecting `Shift+Enter` while focused and pushing `\n` into the buffer manually.

**Breaks if:** (1) A form with multiple TextInput nodes — the second and later inputs never auto-focus on first appearance. (2) A multiline TextInput — pressing Shift+Enter does nothing instead of inserting a newline.

## 2026-04-30 — [FIX] Cmd+Q freezes when any full-screen TUI is running in a terminal pane (PR #454 → alpha)

Two bugs in `deps/egui_term/src/backend/mod.rs` combined to freeze the app on Cmd+Q whenever a full-screen TUI (Claude Code, `btop`, `files`) was running. The 25% CPU ghost + indefinite freeze were separate symptoms from separate root causes.

**Bug 1 — `pty_event_subscription` busy-loop on channel close:** The thread used `loop { if let Ok(event) = recv() }`. When `TerminalBackend` drops and the sender is released, `recv()` returns `Err` immediately. The `if let Ok` silently falls through and the loop spins forever — one full CPU core. Fixed by changing to `while let Ok` so the thread exits when the channel closes. Also changed the `panic!` on a failed proxy send to a `break` — the receiver can legitimately be gone during shutdown.

**Bug 2 — `reap_child` blocking the render thread:** `TerminalBackend::drop` (added in PR #442) called `reap_child()` synchronously, which polls `waitpid(WNOHANG)` 8 × 25 ms then blocks on `waitpid(0)`. Full-screen TUIs don't exit within 200 ms of `Msg::Shutdown` — they restore the terminal first — so drop blocked the render thread per pane. Fixed by spawning `reap_child` on a named background thread (`pty-reap-<pid>`).

**False lead:** Moving `lsof` calls off the render thread (PR #453) did not fix it — reverted.

**Breaks if:** Cmd+Q with a full-screen TUI running freezes the app or requires force-quit.

## 2026-04-29 — [FIX] ai.query hangs forever: invalid model ID + missing config + response delivery race (PR #434)

Three compounding bugs caused every `ai.query` call to hang permanently showing "Waiting for the host's AI broker…" with no error surfaced.

**Bug 1 — Invalid model ID in config template (`google/gemini-flash-2.0`).**
The `CONFIG_TEMPLATE` in `config.rs` had `model_low = "google/gemini-flash-2.0"` which is not a valid OpenRouter model ID (correct: `google/gemini-2.0-flash-001`). OpenRouter stalled the connection rather than returning a 4xx, and `ureq` had no read timeout, so the broker thread blocked forever. Fix: corrected model ID in template; added 30s read + 10s connect timeout via `ureq::AgentBuilder` in `openrouter.rs`.

**Bug 2 — `config.toml` never created on install.**
`PlexiConfig::load()` returned `unwrap_or_default()` silently when the file was absent — never writing the template. `open_config_file()` (the only function that writes the template) is only called when the user explicitly opens their config, never on launch. Fresh install → no `config.toml` → `ai_config = None` → broker returned an error string that was never visible to the user. Fix (PR #434): `PlexiConfig::load()` now writes `CONFIG_TEMPLATE` before parsing if the file doesn't exist, logging at `info`. The `[ai]` section in the template is uncommented with correct default model IDs.

**Bug 3 — Broker error response silently dropped (response delivery race).**
The broker dispatches on a spawned thread and returns immediately on config errors. Because the thread can complete and post to `http_tx` before the Python SDK has registered `_pending_ai[req_id]`, the `AiResponse` event arrives at the `_reader` while the dict is still empty — `q = self._pending_ai.pop(req_id, None)` returns `None` and the response is silently dropped. The `await q.get()` in `ai_query` then waits forever. Fix (PR #434): register `_pending_ai[req_id] = q` **before** emitting the `AiQuery` draw command. Also added `asyncio.wait_for(q.get(), timeout=35.0)` so any future dropped response surfaces as a clear error after 35s.

**Bug 4 — SIGTERM on close (app doesn't exit within 2s).**
When a query is in flight and `Shutdown` arrives, the `ai_query` coroutine is blocked at `await q.get()`. The asyncio event loop can't cleanly finish — app exits only after SIGTERM (2s+ delay). Fix (PR #434): shutdown handler drains `_pending_ai`, cancels all pending waiters with `return_exceptions=True`.

**What NOT to do:** Do not fix `ai.query` hangs by increasing timeouts alone. The core issue was the response delivery race — timeouts only shorten the wait, they don't fix the dropped-response path. Always register `_pending_ai` before emitting the draw command.

**Diagnostic recipe for future hangs:** (1) Check `plexi.log` for `WARN ai broker` — if absent, config is not loaded. (2) Confirm `~/.plexi-alpha/config.toml` exists. (3) Grep for `openrouter: dispatching` — if absent, `AiQuery` never reached the host router. (4) If dispatching appears but no response, check for HTTP 4xx (bad model ID).

**Breaks if:** After a fresh install with no existing `config.toml`, AI queries still hang — `PlexiConfig::load()` didn't write the template on startup.

## 2026-04-29 — [CHANGED] IQ → AI rename + Ollama backend + delete legacy Claude CLI path (PR #433)

Renamed all "IQ" → "AI" across the codebase (92+ sites): protocol types (`IqQuery` → `AiQuery`, `IqResponse` → `AiResponse`), wire strings, capability string (`iq.query` → `ai.query`), module (`src/plexi_iq/` → `src/plexi_ai/`), config section (`[iq]` → `[ai]`), Python SDK (`emit.iq_query` → `emit.ai_query`), and example apps. Added `OllamaBackend` — NDJSON streaming via `/api/chat`, pluggable via `[ai] backend = "ollama"`. Deleted `src/agent_turn.rs` and all `InProcessAgent` code (session persistence, SOUL/MEMORY loading, `claude -p`). Deleted `DrawCommand::LlmRequest` / `PlexiEvent::LlmResponse`. Ledger renamed to `ai-ledger.jsonl`.

**Breaks if:** An installed app still declares `iq.query` in its manifest — skipped at startup with WARN. Any app calling `emit.iq_query()` gets `AttributeError`.
## 2026-04-28 — [FIX] Minimap hidden after Cmd+1-9 workspace switch

**Root cause:** `Action::SwitchContext` (Cmd+1-9) was inlining the workspace switch — directly setting `self.active_workspace = n` and calling `pick_active_context_from_workspace` — bypassing `switch_workspace` and its minimap save/restore logic. Sidebar clicks correctly called `switch_workspace`; Cmd+1-9 did not.

**Fix:** Replaced the inline block in `app/mod.rs` with a call to `self.switch_workspace(n)`. Three lines removed, one line added.

**What NOT to do:** Do not switch workspaces by setting `self.active_workspace` directly anywhere. The only valid path is `self.switch_workspace(n)`. See issue #380 for the structural fix that makes this impossible to bypass at the type level.

**Breaks if:** Pressing Cmd+1 when on a single-window context, then Cmd+2 to a multi-window context, hides the minimap that was previously visible.

## 2026-04-28 — [GOTCHA] confirm_quit / confirm_close wiring — diagnosis correct, approach superseded

**Root cause found:** On macOS, pressing Cmd+Q fires two simultaneous events: (1) a keyboard event consumed by `keys.rs` → `Action::Quit` → triple-tap logic, and (2) a viewport `close_requested` event generated by NSApp's `applicationShouldTerminate`. The `close_requested` handler at `app/mod.rs` ran unconditionally and called `save_workspace()` without sending `ViewportCommand::CancelClose`, so the OS close always won the race — the triple-tap overlay would briefly appear but the app quit before the user could press Cmd+Q a second time.

**What was changed in this session:** `close_requested` handler updated to send `CancelClose` when `quit_confirm_required && quit_press_count > 0` (keyboard quit flow in progress). X-button close kept working by allowing it through when `quit_press_count == 0`. `~/.plexi-alpha/config.toml` updated to show both `confirm_quit = true` and `confirm_close = true` uncommented. Legacy `quit_confirm` field removed from `BetaConfig` (was `[beta]` section, now gone). Template source updated to match.

**Why this was superseded:** The fix was correct for the Cmd+Q bypass but a broader refactor of the confirmation/close architecture was done by a separate agent. Do not re-apply this patch on top of that refactor — the close_requested handler and the Action::Quit dispatch may both have changed shape.

**What NOT to do:** Do not add `unwrap_or(false)` defaults for `confirm_quit` / `confirm_close` — these should default to `true`. Do not re-introduce `beta.quit_confirm` as a fallback.

## 2026-04-28 — [CHANGED] Minimap fade machinery removed — static colors only

**Why:** The minimap was flickering between two brightness states. Root cause: `alpha_mult()` returned either `1.0` (recently active) or `0.15` (faded), with no smooth interpolation. Any interaction reset `last_activity` and caused a visible binary jump. The fade timer approach was fragile — it required `request_repaint_after(50ms)` scheduling, `on_activity()` call sites across 7 locations, and produced confusing UX.

**What was removed:** `last_activity: Instant`, `FADE_START_SECS`, `FADE_DURATION_SECS`, `FADED_ALPHA` constants, `apply_alpha()` helper, `alpha_mult()`, `needs_repaint()`, `is_faded()`, `on_activity()` — all deleted. The `state: &MinimapState` parameter was removed from `render_minimap`. All 7 `on_activity()` call sites across `mod.rs` and `workspace.rs` removed. The `request_repaint_after(50ms)` scheduling removed from `overlays.rs`.

**What remains:** `MinimapState` is now `{ visible: bool }` only. Colors are used directly from the `Colors` theme struct every frame — no alpha multiplication. The minimap appears and disappears instantly (no fade-in/fade-out animation).

**Breaks if:** The minimap still flickers between bright and dim states, or fades out after a period of inactivity.

## 2026-04-28 — [FIX] Minimap trail stale after horizontal creation + dim-on-workspace-switch

**Bug 1 — Trail indicator stays on source page after horizontal page creation**

The "trail" indicator in the minimap is controlled by `last_page_x_per_row[row] == page.grid_x && !is_active`. When navigating TO an existing page, `navigate_page` fires a post-navigation `insert(dest_row, dest_x)`, clearing the trail from the source. But when navigation FAILS and `create_page_at` runs instead (no page existed to the right), that post-insert never fires — `last_page_x_per_row[row]` stays at the old source x, rendering the source cell as trail even though you've moved past it.

Fix: in `navigate_or_create_page`, after `create_page_at`, call `last_page_x_per_row.insert(ty, create_x)`. This registers the created page's position as the current row's breadcrumb, matching what `navigate_page` would have done had the page already existed.

**The invariant that must always hold:** `last_page_x_per_row[row]` should equal the active page's `grid_x` when you're in that row. Trail only appears in rows you've LEFT (other rows), showing where you were before going vertical. This invariant is maintained by: navigate_page post-insert (for existing pages) + create post-insert (for new pages). Any new code path that changes `active_context` without updating `last_page_x_per_row` will produce stale trails.

**Bug 2 — Minimap renders dim immediately after switching to a workspace where it should be visible**

`switch_workspace` restores `minimap.visible = true` for the target workspace, but `minimap.last_activity` carries over from the old workspace's last navigation. If that was > 3 s ago, `is_faded()` returns true immediately and the minimap renders at 15% opacity. The user then navigates, `on_activity()` fires, and the minimap jumps to 100% — appearing to "randomly become brighter."

Fix: in `switch_workspace`, after `minimap.visible = true`, call `minimap.on_activity()` to reset the fade timer. The minimap starts at full opacity on workspace switch and fades naturally after 3 s of inactivity.

**Breaks if:** moving right to a newly created page still shows the old page highlighted as the trail indicator, or switching to a multi-page workspace shows a dim minimap that only brightens on first navigation.

## 2026-04-27 — [FIX] Vertical navigation regression + minimap visibility per workspace

**Bug 1 — Vertical navigation broken (Cmd+Shift+K goes to wrong page)**

Root cause: a prior change to `navigate_page` replaced the `last_page_x_per_row` preference system with `min_by_key(grid_x)` (always go leftmost). This was intended to fix creation semantics (new rows start at col 0) but incorrectly applied the same logic to navigation of *existing* pages — destroying the column-memory that returns you to the page you came from.

The correct invariants are distinct:
- **Navigation** (`navigate_page`): go to the closest-by-column page in the target row, preferring the last-visited column for that row via `last_page_x_per_row`.
- **Creation** (`navigate_or_create_page`): when no page exists in the target row, create at column 0.

These two behaviors live in separate functions for a reason. Changing `navigate_page` to implement creation semantics broke navigation. The fix: restore the original `preferred_x` logic in `navigate_page`; the creation `create_x = 0` change in `navigate_or_create_page` was already correct and was left untouched.

**Systemic fix — pure function + unit tests:**

Extracted the navigation target search into `find_navigation_target()` — a free function with no access to `&mut PlexiApp`, taking only data slices. This function is independently unit-testable. 12 tests now encode the exact contract: same-column preference, `last_page_x_per_row` memory, closest-by-distance fallback, boundary conditions, workspace isolation. Any future regression in navigation logic immediately fails a test — the bug that prompted this entry would have been caught at `cargo test` time.

**Do NOT put creation logic into `navigate_page` or `find_navigation_target`.** Those functions are navigation-only. Creation policy belongs exclusively in `navigate_or_create_page`.

**Bug 2 — Minimap visibility not preserved across workspace switches**

Root cause: `minimap.visible` was a single global bool. Switching workspaces always inherited whatever the previous workspace left it at.

Fix: `minimap_visible_per_workspace: HashMap<u64, bool>` stores saved state per workspace id. All workspace switches now go through `switch_workspace()` which saves the old workspace's visibility before switching and restores (or defaults to `page_count > 1`) for the new workspace. `delete_workspace` and `delete_context` use the same restore logic. `switch_workspace` is now the single required path for workspace switching — inlining `active_workspace = i + pick_active_context_from_workspace` elsewhere bypasses the save/restore.

**Breaks if:** Cmd+Shift+K from window 4 goes to window 1 instead of the column-matched window above it. Or: switching contexts resets the minimap to hidden even when it was previously shown.

## 2026-04-27 — [FIX] Minimap clicks passing through to pane behind it

**Root cause:** In egui, *painting* and *allocating for input* are two entirely separate operations. `painter().rect_filled(panel_rect, ...)` draws pixels but makes no claim on pointer events for that area. Only `ui.allocate_rect(rect, Sense::...)` creates an interactive region that participates in egui's hit-test system. The minimap panel background, title, border, and cell gaps were all drawn visually but never allocated — so any click that didn't land on an exact cell rect fell through to the tile widget behind the overlay, which switched pane focus.

**Fix:** Added `ui.allocate_rect(panel_rect, egui::Sense::hover())` at the top of `render_minimap`, before any cell rendering. egui's area ordering ensures a `Foreground` Area wins hit-testing over `Middle`/`Background` areas, so once the full panel rect is claimed, the pane below receives nothing.

**What this reveals — the deeper architectural asymmetry:**

Plexi has a principled, enforced model for keyboard input ownership (`FocusLayer` stack — non-pane layers push onto it; panes only see keyboard events when nothing else owns the stack). There is no equivalent for pointer events. Each overlay handles its own pointer input ad-hoc. Whether a click is consumed depends entirely on whether the developer remembered to allocate a Sense — an invisible, easy-to-forget requirement with no compile-time enforcement.

**The structural fix planned:** Introduce `OverlayPanel`, a thin wrapper around `egui::Area` that *always* allocates its bounding rect before forwarding to the inner render closure. Using it makes it impossible to render a floating panel without also claiming its input region — you'd have to actively break the abstraction to recreate this bug. All interactive overlays (minimap, notification modal, command palette, confirm-close) migrate to `OverlayPanel`. Optionally, `OverlayPanel` populates a frame-scoped `PlexiApp::overlay_claims: Vec<Rect>` that pane/tile code can consult — the pointer analogue to `FocusLayer`.

**Do NOT:** reach for `Sense::click()` on the background rect to also handle panel-level clicks — `Sense::hover()` is sufficient to block passthrough. `click()` would also suppress the cell `ui.interact()` responses that return the clicked context index.

**Breaks if:** clicking anywhere in the minimap panel (background, title, border) still switches pane focus in the content area behind it.

## 2026-04-27 — [FUTURE] Per-context minimap: open bugs after this session

Three bugs remain after this session. Documenting clearly so the next session doesn't repeat the failed approaches.

**Bug 1 — Minimap count off by one (shows N+1 cells, needs two Cmd+N to appear)**

Root cause attempt 1: Added home context as a synthetic cell at (0,0) in `render_minimap`, alongside spatial pages at (1+,0). Threshold lowered to 1 spatial page. Result: minimap showed 3 cells after 2 Cmd+N (correct = home + 2 spatial), but didn't appear after 1st press. The reason for the delay is still unknown — `minimap.visible = true` + `on_activity()` are called in `create_page_at` which runs on Cmd+N. **Reverted.**

Likely correct approach: the minimap should show the home cell (sidebar context itself) as cell (0,0). Spatial pages should continue starting at x=1 for row 0 (so home at 0 and pages at 1+). The rendering bug probably lies in how `render_minimap` determines the panel bounding box when home is included — max_x based only on spatial pages may produce a panel too narrow to show both home and page(1,0) without a gap. The panel bounding box must include column 0 (home) explicitly, regardless of spatial page positions.

**Bug 2 — Grid gaps after deletion (pages don't collapse inward)**

When a spatial page is deleted, sibling pages to its right keep their `grid_x` unchanged. The minimap renders each page at its stored `grid_x`, leaving empty columns where deleted pages were.

Correct fix: on deletion of a spatial page at `(del_x, del_y)`, decrement `grid_x` of every sibling with `parent_context_id == parent_id && grid_y == del_y && grid_x > del_x`. Same logic for rows when a full row is deleted.

**Bug 3 — Cmd+W on last pane stops here (no blank canvas)**

Current state: Cmd+W on the last pane does nothing (no quit, no context delete — correct). But the intended behavior is: closing the last pane leaves an empty context (blank canvas) and the user presses Cmd+N to create a spatial page. This requires the UI to handle a context with zero panes gracefully — currently untested. The `execute_close_pane` function currently just bails on the last pane; the blank-canvas state and its rendering path need to be designed and built.

**What was changed this session and kept:**
- `create_page_at`: `minimap.visible = true` + `on_activity()` on first spatial page (threshold was 2, now 1)
- `execute_close_pane`: removed the quit path (last pane no longer closes app)
- Spatial pages now start at `grid_x = 0` (was 1 for row 0), eliminating the phantom gap column
- Per-context minimap with `context_id`/`parent_context_id` filtering is stable

**What was tried and reverted:**
- Home cell at (0,0) in `render_minimap` with `sidebar_context_idx` extra param — count was wrong and minimap didn't show on first press
- `spatial.rs` home fallback (navigate left from x=0 → sidebar context) — removed because spatial pages now start at x=0, making left-from-first-page go to tx=-1 which already returns early; the fallback was unreachable

## 2026-04-26 — [FIX] Four regressions from spatial workspace commit → alpha

Mouse blocked app-wide: `draw_minimap_overlay` allocated the full screen with `Sense::hover()` inside a `Foreground` Area, which captured all pointer events before any pane or sidebar widget could see them. Fixed by removing the full-screen `allocate_exact_size` — the Area now auto-sizes to the minimap panel cells via `ui.interact()` calls inside `render_minimap`.

`Cmd+Shift+M` toggled opacity instead of show/hide: `MinimapState` had `override_visible: bool` intended to pin full opacity; `toggle()` flipped it, making the minimap brighter/dimmer but never hiding it. Renamed to `visible: bool` (default `true`), `toggle()` now flips show/hide, `draw_minimap_overlay` returns early when `!visible`.

Spatial pages in sidebar: sidebar loop iterated all contexts with no filter. Added `if self.contexts[i].spatial { continue; }` guard. Pages (`Cmd+N` / `Cmd+Shift+N`) now set `spatial: true`; sidebar contexts set `spatial: false`.

Minimap covering `?` button: `INSET = 8.0` left the panel overlapping the toolbar and shortcuts button (~44px from top). Split into `INSET_TOP = 52.0` / `INSET_RIGHT = 16.0`.

Also: removed `#[serde(default)]` from `spatial` in `SavedContext` — old workspace files without the field will fail to parse, trigger the existing backup-rename logic in `WorkspaceFile::load()`, and start fresh. No backward compat shim needed.

**Breaks if:** clicking in panes or sidebar does nothing (mouse regression), or `Cmd+Shift+M` makes the minimap dimmer rather than hiding it, or spatial pages appear as tabs in the sidebar.

## 2026-04-26 — [CHANGED] Spatial 2D page grid + minimap overlay (PR → alpha)

Added `grid_x / grid_y` to `Context` and `SavedContext` (`#[serde(default)]` for backward compat). `Cmd+N` / `Cmd+Shift+N` now create pages on the grid instead of splitting panes — splits moved to `Cmd+\` / `Cmd+Shift+\`. `Cmd+Shift+H/J/K/L` navigate between pages (was LateralFocus — no free chord available so repurposed). Minimap overlay (`src/minimap.rs`) renders in top-right corner, fades after 3 s idle, pinned by `Cmd+Shift+M`. Page creation and spatial helpers live in `pane_ops/workspace.rs` (co-located with `new_context` for access to `pub(super) create_single_pane_tree`). Navigation-only helpers in `src/spatial.rs`.

Decision: `LateralFocus` variant removed entirely (was never constructed after keybind repurposing — keeping dead code violates project rules). Apps that depended on lateral-focus shortcuts will need to adjust if any existed; none are known.

**Breaks if:** `Cmd+N` opens a pane split instead of a new page, or `Cmd+\` does nothing, or the minimap doesn't appear after creating 2+ pages.

## 2026-04-26 — [FIX] Render event coalescing — try_send drops during slow Python startup (#368 → PR #378)

Root cause: `sync_channel(1024)` + `try_send` filled with `PlexiEvent::Render` bursts during Python import phase. When the channel was full, every subsequent render event was silently dropped → apps appeared frozen, scroll (#371) never re-rendered. The earlier stdin-writer-thread fix (`cbf2799`) moved writes off the GUI thread but left the bounded channel and `try_send` in place.

Fix: introduced `StdinItem` enum (`Event(String)` | `FlushRender`). Channel is now unbounded (`mpsc::channel`). Render events are coalesced: latest payload stored in `Arc<Mutex<Option<String>>> render_slot`; a single `FlushRender` token is queued via `Arc<AtomicBool> render_in_queue` guard. Writer thread resets the flag *before* draining the slot so a concurrent `send_event` can re-queue without a race. Non-render events pass through in-order, never dropped. #371 (scroll broken) resolves as a downstream effect.
**Breaks if:** apps start up but don't respond to key events or scroll after the first render.

## 2026-04-26 — [FIX] Secret not persisted after granting via prompt — re-prompt loop every launch (#372 → PR #377)

Root cause: `show_prompt_modal` in `src/process_app/prompts.rs` sent `PlexiEvent::SecretValue` to the app on grant but never called `MacKeychain::set()`. Every new launch hit an empty keychain and re-prompted.

Fix: `persist_granted_secret(workspace_root, app_id, key, value, store: &dyn SecretStore)` called immediately after grant. Routing mirrors `routing.rs`: explicit route in `workspace.toml` + `secrets.toml` → workspace-namespaced account (`plexi:<ws-id>:<friendly>`); no route / fallback=true → `plexi:user:<key>`. Guarded by `#[cfg(target_os = "macos")]` using `MacKeychain::new()`. Tests cover no-workspace, explicit-route, fallback, and deny paths.
**Breaks if:** granting a secret prompt still re-prompts on the next app launch.

## 2026-04-26 — [FIX] Agent workspace modal TextInput ignores keyboard input (#370 → PR #376)

Root cause: auto-focus condition `!r.has_focus() && modal.task.is_empty()` was checked every frame. `request_focus()` is deferred one frame in egui, so `has_focus()` is false when the request fires — the condition re-triggers every frame and fights the user's own focus choices. Once the user typed anything, `task.is_empty()` also permanently prevented re-focus after a tab.

Fix: `focus_initialized: bool` field on `AgentWorkspaceModal`, set `false` in `open()`. `request_focus()` called exactly once on the first render frame; after that the user's explicit focus choices are not overridden.
**Breaks if:** opening the modal and typing immediately produces no input in the repo path field.

## 2026-04-26 — [FIX] Agent workspace palette commands give no feedback when CLI not installed (#369 → PR #375)

Root cause: `spawn_agent_workspace()` in `command_palette.rs` called `open_agent_workspace_pane()` without first checking `cli.is_installed()`. On failure it only logged — no host notification, so the user saw nothing.

Fix: `is_installed()` checked at the top of `spawn_agent_workspace()`; calls `push_host_notification` on both the not-installed path and the `open_agent_workspace_pane` error path. Mirrors the pattern already used in `spawn_agent_workspace_from_modal`.
**Breaks if:** clicking "New Agent Workspace: Claude Code" with Claude Code uninstalled produces no visible feedback.

## 2026-04-26 — [FIX] AppBar descender clipping — title text bottom pixel cut off (#373 → PR #374)

Root cause: `text_y = y + (self.BAND_H - self.TITLE_SIZE) / 2.0 - 1.0` in `sdk/python/plexi_sdk/ui.py`. The `-1.0` nudge shifted the text 1px toward the top clip boundary, clipping descenders (g, p, y, etc.) at 16pt.

Fix: removed the nudge — `text_y = y + (self.BAND_H - self.TITLE_SIZE) / 2.0`. The original nudge comment described it as "empirical compensation for proportional-font descent bias"; it was wrong and the true centre is correct.
**Breaks if:** AppBar title sits visibly off-centre vertically.

## 2026-04-26 — [FIX] GUI hang when spawning apps — stdin write blocked egui main thread

Root cause: `ProcessApp::send_event` called `stdin.write_all()` directly on the egui render loop thread. During the "starting" window (Python process importing modules, not reading stdin yet), pipe buffer fills fast. The first `Render` event write blocks indefinitely → macOS hang report → forced kill. Symptom: app shows "starting" pill then the entire host dies.

Fix: background writer thread owns `ChildStdin` and blocks on writes there. GUI thread pushes to a bounded `sync_channel(1024)` via `try_send` — non-blocking in all cases. `cbf2799`.

## 2026-04-26 — [GOTCHA] Info.plist.fragment must NOT include full plist wrapper

`cargo-bundle 0.9.0` handles `osx_info_plist_exts` by doing a raw text insert inside the `<dict>` of the generated Info.plist. The fragment file must contain only bare key-value pairs — **no** `<?xml?>` declaration, no `<!DOCTYPE>`, no `<plist>` or `<dict>` wrappers. Including the full plist boilerplate (as the #277 sub-agent wrote) embeds a second XML document inside the `<dict>`, producing malformed XML that macOS rejects as "damaged or incomplete" at launch. Fix: `assets/Info.plist.fragment` is now just the two `<key>`/`<string>` lines. Re-sign with `codesign --force --deep --sign -` after install.

## 2026-04-25 — [CHANGED] Agent-as-app foundation — manifest type field + protocol variants + broker widening (#285 part 1 → alpha)

First slice of #285 (v3.3 P1 headline "Agent-as-app"). Lands the wire and schema additions that the host integration in part 2 will consume; does NOT yet spawn subprocess agents into `Pane::Agent` or ship the SDK Agent class. Scoped down deliberately to keep the PR reviewable — the host integration ripples through `process_app/routing.rs`, `agent_pane.rs`, `pane.rs`, and the SDK simultaneously, and is its own commit.

**Manifest schema (`src/app_registry.rs`)** — required `[app] type = "app" | "agent"` field on every manifest. Discipline matches `schema_version` (#308 Phase 2): no `serde(default)`, no fallback to `"app"`, missing field → loud parse error. New `ManifestType` enum (`App` | `Agent`). Optional `[launch] system_prompt: Option<String>` for agent manifests; the host forwards it to the agent subprocess via `PlexiEvent::AgentInit` once the host integration lands. All 18 example manifests migrated to add `type = "app"`. Inline manifest fixtures in `src/install.rs` test helpers also migrated.

**Protocol additions (`src/app_protocol.rs`)** — three new variants, all required-field shape, no `serde(default)`:
  - `PlexiEvent::AgentInit { system_prompt: Option<String> }` — sent once at agent startup with the manifest's `system_prompt`. Only delivered to `type = "agent"` panes. `Option` is explicit on the wire (`null` for unset).
  - `PlexiEvent::UserMessage { text }` — sent when the user submits text in the host-rendered conversation input box. Only delivered to agent panes.
  - `DrawCommand::AppendConversation { role, content }` — agent emits one per logical turn; host renders into the conversation history surface. `role` accepted as a string (`"user" | "assistant" | "tool" | "system"`) for forward-compat with future role kinds; unknown roles render as plain text.

**Broker widening (`src/plexi_iq/{broker,loop,backend/{mod,anthropic_api}}.rs`)** — lifted the `flatten_messages` stop-gap from #284. `LlmRequest` now carries `messages: Vec<IqMessage>` directly (the structured Anthropic Messages shape) plus `system: String`. `AnthropicApiBackend::stream_native` translates each `IqMessage` to a `MessageBuilder`-built `Message` with the matching `MessageRole`; empty `messages` is a loud error (`"LlmRequest.messages is empty"`), unknown role values likewise. `turn_loop::run_turn` widened: `messages: Vec<IqMessage>` instead of `prompt: impl Into<String>`. Multi-turn agent conversations now flow natively — no more `[assistant previously]:` prefix joining.

**Out of scope deliberately (filed as follow-up):**
  - Host integration: spawning subprocess agents into `Pane::Agent` with conversation UI scaffolding from `agent_pane.rs` backed by subprocess `AppendConversation` events (vs. the legacy hardcoded `agent_turn` loop).
  - SDK `Agent` base class with the `on_user_message(text) -> str` callback pattern.
  - POC `examples/agent-tester/` end-to-end agent.
  - `plexi app new --type agent` scaffolder.
  - Deletion of `agent_turn.rs` / `agent_pane.rs` legacy in-pane turn loop.

Test-first: 11 new tests across `app_protocol::tests` (5), `app_registry::tests` (5), `plexi_iq::broker::tests` (1, replacing the deleted `flatten_messages_joins_user_turns`). All 114 pass; clean `cargo build --release`.

**Breaks if:** any existing manifest loads after this PR without `type = "app"` set; `cargo test --bin plexi` reports any test in `app_protocol::tests::user_message_*`, `agent_init_*`, `append_conversation_*`, or `app_registry::tests::manifest_with_type_*` failing; `LlmRequest.messages` empty no longer surfaces a stream error containing `"messages is empty"`; or an agent manifest's `[launch].system_prompt` field is silently ignored at load (verify by checking `~/.plexi-alpha/plexi.log` — every `launching '<id>'` line should now also log `type=...` and `system_prompt=...`).

## 2026-04-26 — [CHANGED] `iq.query` brokered capability — first v3.3 milestone PR (#284 → alpha)

Opens the v3.3 milestone (Agents as First-Class Citizens). Three new wire types in `src/app_protocol.rs`: `DrawCommand::IqQuery { request_id, model_tier, system, messages, tools }`, `PlexiEvent::IqResponse { request_id, content, tokens_in, tokens_out, error }`, and the `ModelTier` enum (`low | medium | high`). All fields required — no `serde(default)`. Adds `IqQuery` to the `Capability` enum (string `"iq.query"`); manifest validator and `from_capability_strings` recognise it.

New `src/plexi_iq/broker.rs` module owns the brokered path. `IqBroker` trait + `LiveIqBroker` (production) + `CannedBroker` (tests). `route_command` synchronously emits the gate-denied response when the manifest doesn't declare `iq.query`, and otherwise spawns a worker thread that calls `broker.dispatch()` and forwards the response onto `http_tx`. `LiveIqBroker` resolves `ANTHROPIC_API_KEY` per-call through the existing workspace-scoped secrets store, maps `ModelTier` to a concrete model id (`low → claude-haiku-4-5`, `medium → claude-sonnet-4-5`, `high → claude-opus-4-5`), and routes through `AnthropicApiBackend::with_model` + `turn_loop::run_turn` so we re-use the existing streaming + ledger path rather than forking a second LLM client.

`LedgerRow` extended with optional `app_id` and `model` (skipped when None so legacy `Pane::Agent` rows aren't polluted) plus a required `cost_cents: u64` (computed at row construction so dependents don't have to). `LedgerRow::with_attribution` is the new call site for `iq.query`; `LedgerRow::new` stays for the legacy in-process turn loop. The broker appends a row on every successful dispatch — failures are logged but never propagate (a billing miss must never break the conversation).

`plexi_iq` was on disk but never registered as a module; added `mod plexi_iq;` to `main.rs`. Module also gains `pub mod broker`. The existing `src/process_app/routing.rs` `LlmRequest` handler (legacy `llm` capability via raw `ureq` Anthropic call) is left in place — `iq.query` is the v3.3 successor; #285 will eventually retire `LlmRequest`.

Python SDK adds `Emitter.iq_query(model_tier, system, messages, tools=None) -> IqResponse`, an `IqResponse` dataclass, and a `CapabilityDeniedError` exception (raised when the host returns "capability denied" — apps that want to handle the gate-denied path explicitly can `except CapabilityDeniedError`). The blocking pattern matches `secret_get` / `http_request` / `measure_text` — UUID `request_id`, per-request `queue.Queue`, drained by the App's stdin event loop on `iq_response`. `tools` non-empty is rejected at the broker until v3.4 ships tool dispatch — explicit refusal beats a silent drop.

Two POC apps land alongside the protocol work (mandatory per orchestration rule):
- `examples/iq-query-test/` — manifest declares `iq.query`. Text input + l/m/h keys to dispatch at each tier; renders content + token counts on success, red error card on failure.
- `examples/iq-query-denied-test/` — manifest declares no capabilities. Press `s` to verify the gate fires; the `CapabilityDeniedError` arrives within a frame.

Test-first: 14 new tests across `app_protocol::tests` (4), `app_permissions::tests` (2), `plexi_iq::broker::tests` (4), `plexi_iq::ledger::ledger_tests` (2), `process_app::iq_tests` (2). The `process_app` tests inject `CannedBroker`/`PanicBroker` into the `iq_broker` field to drive the routing layer deterministically — no real LLM calls.

**Out of scope for this PR (deliberately):** #285 agent-as-app refactor (the existing `Pane::Agent` hardcoded LLM still uses `agent_turn::run_turn` directly — replacing that with subprocess agents calling `iq.query` is the next dispatch); tool-use loops at the broker level (apps drive their own loops by re-calling `iq_query`); cost-cap enforcement / rate limiting beyond what the existing ledger does; streaming responses (spec calls for one `IqResponse` per query).

**Breaks if:** an app whose manifest does NOT declare `iq.query` calls `emit.iq_query(...)` and gets back content instead of `CapabilityDeniedError`; an app declaring `iq.query` calls into Low/Medium/High tier and the response model doesn't differ between tiers (verify by asking the same prompt at Low and High — the wording should differ); `~/.plexi-alpha/ledger.jsonl` doesn't gain a new line per query with `app_id`, `model`, `tokens_in`, `tokens_out`, `cost_cents`; or `ANTHROPIC_API_KEY` ever appears verbatim in the log file or any app's stderr.

## 2026-04-26 — [CHANGED] App package manager — top-level `install`/`uninstall`/`update`/`list`, manifest `schema_version`, bundled core pack (#308 Phase 2 → alpha)

Phase 2 of the v3.2 headline. Adds a required `schema_version: u32` field to every `manifest.toml` (no `serde(default)` — missing → loud parse error; greater than `MANIFEST_SCHEMA_VERSION = 1` → install refused with a clear message), a new `packs.rs` module that parses `pack.toml` (requirements.txt-style app list with a tagged source-spec scheme: `github:owner/repo`, `git+https/ssh/file://...`, `local:<example-name>`), and a new `install.rs` that owns the install/uninstall/update flow plus a `Cloner` trait (mock for tests, `GitCloner` shells out to `git` in prod — no `git2` dependency).

Five new top-level CLI subcommands route through `cli::install_cli` / `install_pack_cli` / `uninstall_cli` / `update_cli` / `list_cli`. `parse_workspace_path_arg::SUBCOMMANDS` was extended with `install`, `uninstall`, `update`, `list`, `pack` so `plexi install foo` isn't misinterpreted as a workspace-path adoption. Existing `plexi app install` (the older github-shorthand-or-URL path that builds Rust apps) stays in place — top-level `plexi install` is the new git-based flow.

The bundled core pack lives at `packs/core.toml`, embedded via `include_str!`, and uses `local:<example-name>` sources that copy from the compile-time-baked `examples/` tree (re-uses `include_dir!` already in deps for `config::ensure_profile_initialized`). On every launch the host calls `apply_core_pack_if_empty` against the channel apps dir; idempotent, no-op when non-empty. Sources point at bundled examples rather than a yet-to-exist `plexi-apps` org repo to keep Phase 2 fully offline-first; once the org repo exists, swap `local:` for `github:` in `packs/core.toml`.

Install staging uses a hand-rolled `.tmp-install-<pid>-<nanos>/clone` sibling dir inside the apps dir (same FS for atomic `rename`); `tempfile` is dev-only so we don't pull it into the release build for one path. Manifest is read out of the staging dir BEFORE the rename so the canonical `manifest.app.id` keys the final dest — the URL's repo name is never trusted. Failed clones cleanup via early-return helper.

POC demo: extended `examples/workspace-config-tester/` (Phase 1's POC) to also walk `~/.plexi-<channel>/apps/*/manifest.toml` and surface every installed app's `id`, `version`, and `schema_version` in a Card. Hit `r` to reload after `plexi install <repo>` and the new app appears without a host restart. ~25 lines of additions; cheaper than a separate proposal doc.

All 15 example manifests migrated to add `schema_version = 1`. 23 new tests across `app_registry::tests` (3), `packs::tests` (10), `install::install_tests` (4), `install::pack_tests` (3), `install::core_pack_tests` (3). Test-first: failing tests written before each code path, including `Cloner` mock pattern lifted from `workspace_secrets::SecretStore`.

**Breaks if:** `plexi-alpha install <real-public-git-url>` doesn't produce `~/.plexi-alpha/apps/<id>/` with the manifest's id (not the repo's URL slug); or `plexi-alpha install` of a manifest with `schema_version = 99` succeeds instead of erroring with "newer than this Plexi build supports"; or `plexi-alpha uninstall <never-installed-id>` succeeds silently instead of erroring; or `plexi-alpha list` shows nothing on a fresh `~/.plexi-alpha/apps/` (core-pack auto-apply on empty); or running `plexi-alpha install --pack core` twice in a row second-time-installs the same apps instead of reporting "up-to-date"; or any existing example loses its `schema_version = 1` line and silently fails to load with a missing-field parse error.

## 2026-04-26 — [CHANGED] Workspace model — config merge, path-arg adoption, outside-workspace badge, auto `.gitignore` (#308 Phase 1 → alpha)

Phase 1 of the v3.2 headline issue. `<workspace_root>/.plexi/config.toml` now overlays the global `~/.plexi-<channel>/config.toml` on a per-field basis — every field is `Option<T>`, project values win, unset project fields preserve globals. Implementation lives entirely in `src/config.rs::PlexiConfig::load_with_workspace`; the existing `Config` shape was already all-Option so the overlay is a one-pass field walk with no struct refactor and no `serde(default)` tricks. Both `main()` (for log-level resolution) and `PlexiApp::new` (for the rest of the host) now route through `load_with_workspace(active_workspace_root().as_deref())` so a single source of truth feeds everything downstream.

`plexi <path>` is parsed in `main()` via a small `parse_workspace_path_arg` that walks up to find the nearest `.plexi/` ancestor — re-uses the `app_registry::resolve_workspace_root` walker already in tree. On adoption we both record the explicit root via `config::set_adopted_workspace_root` and `chdir` into it, so all the existing "look up from CWD" code paths (`AppRegistry::load`, event log, default pane cwd) stay correct without per-callsite plumbing. Bare `plexi` is unchanged. A bad path (no `.plexi/` ancestor) errors and exits 1 before the GUI starts.

Status-line indicator threads `workspace_root: Option<PathBuf>` into `PlexiBehavior`, then into `render::terminal_pane::render`. The check is host-only: `get_pid_cwd(child_pid).starts_with(workspace_root)` after canonicalizing both sides (macOS `/var → /private/var` would otherwise false-positive). When out-of-tree we paint a small amber "↗ outside workspace" badge into the existing name-bar strip; the strip widens to fit the badge when no name is set. No protocol changes.

`plexi workspace init` now also writes `<root>/.plexi/.gitignore` with `secrets.toml` + `cache/` — but only when the file is absent. The check-then-write avoids stomping a user's edits on subsequent `init` runs, and the body of the auto-generated file says so explicitly. Touched `workspace_secrets::init_workspace`; the `init_workspace_creates_workspace_and_secrets_files` test from #322 still passes verbatim.

POC app: `examples/workspace-config-tester/` — one pane, `r` to reload, surfaces the resolved workspace root, the workspace UUID, and a few representative `[log]/[theme]/font_size` keys read from `<root>/.plexi/config.toml` so the user can drop a project config and watch it pick up.

**Breaks if:** dropping `[log] level = "debug"` into a workspace's `.plexi/config.toml` has no effect on the host's log level; or `plexi-alpha /path/to/workspace` doesn't adopt the path (CWD stays elsewhere, registry skips workspace apps); or `plexi-alpha /tmp` (no `.plexi/` ancestor) launches the GUI silently instead of exiting 1 with a clear error; or `plexi-alpha workspace init` does not produce `.plexi/.gitignore` with `secrets.toml` and `cache/`; or re-running `workspace init` overwrites a user-edited `.plexi/.gitignore`; or a terminal whose CWD is outside the workspace root never shows the `↗ outside workspace` badge.

## 2026-04-25 — [CHANGED] Cmd+N / Cmd+Shift+N split-with-mirror + Shift+Cmd+H/J/K/L lateral focus (#306 → alpha)

Cmd+N was previously bound to `Action::NewContext` (create a new context tab). It's now `SplitRight` — split the focused pane to the right with a new pane that mirrors the focused pane's type (terminal → terminal, app → fresh instance of same app, agent → new agent). Cmd+Shift+N adds `SplitDown` with the same mirror rule. Shift+Cmd+H/J/K/L adds explicit `LateralFocus(Direction)` bindings; they share the geometric direction-finder used by Cmd+H/J/K/L (one impl, two surfaces). The new-context shortcut is intentionally OUT OF SCOPE for this PR and was not re-bound — sidebar "+" and `new_context()` are unchanged. Cmd+T still maps to `NewTab` (within-context tab creation). The geometric direction-finder was refactored: pure-geometry logic extracted into `find_in_direction_geometric` (takes `from_rect` + `(T, Rect)` candidates iterator) so it's unit-testable without spinning up egui. Spec: tier 0 = candidates whose perpendicular axis range overlaps the source's; primary distance = source center to candidate's nearest edge along the requested axis; secondary tie-break = perpendicular-axis center distance. App-mirror dispatch reuses `launch_app_by_id_with_layout` with `split_v`/`split_h` layout overrides instead of constructing AppPane manually — keeps a single launch-with-permissions path. Rejected alternatives: per-launch ad-hoc AppPane construction (duplicated permissions/group/share logic); a separate `LateralFocus` direction-finder distinct from `Navigate` (would diverge over time).

**Breaks if:** Cmd+N opens a sidebar/new-context tab instead of splitting the focused pane to the right; Cmd+Shift+N does anything other than split below; Shift+Cmd+L on a pane with a right neighbor lands on the wrong neighbor; splitting an app pane doesn't produce a fresh instance of the same app; existing Cmd+D / Cmd+Shift+D split shortcuts regress; sidebar "+" no longer creates a new context (would mean `new_context()` was accidentally affected).

## 2026-04-25 — [CHANGED] Workspace-namespaced secret routing (#322 → alpha)

Three layers landed: (1) Keychain entries under `plexi:<workspace-id>:<friendly>` (workspace) or `plexi:user:<friendly>` (cross-workspace fallback), workspace ID a UUID stored in `<root>/.plexi/workspace.toml`; (2) app manifests declare `[secrets] X = { required = ..., description = "..." }` (no serde-default on `required`); (3) `<root>/.plexi/secrets.toml` is the router with required `fallback` field plus `[apps.<id>]` and `[default]` route tables. Resolution is a 4-step pure function in `workspace_secrets::resolve`: app-route → default-route → user-scope-on-fallback → hard-miss-or-prompt. New `SecretStore` trait isolates Keychain so tests use `InMemoryKeychain` and never hit `security` CLI. CLI: `plexi workspace init`, `plexi secret {set,list,delete} <friendly>` are workspace-aware (replaces the legacy plexi-run-style flat layout — no compat shim).

**Migration:** `secrets-index.json` schema flipped from `Vec<SecretEntry>` to `Vec<String>` (full account names). Legacy entries are re-stored under `plexi:user:<key>` on first GUI launch via `migrate_legacy_global_secrets`; idempotent, logged per migration. Old global Keychain entries (`plexi-run/<dir>/<key>`) are read once during migration and not deleted — they remain dormant.

**Breaks if:** `plexi-alpha workspace init` doesn't create a `.plexi/workspace.toml` with a valid UUID; or two directories with `init` then different `[apps.<id>]` routes for the same canonical secret return the same Keychain value; or `fallback = false` with no route silently falls through instead of throwing the `hard_missing` SecretDenied event; or an app declaring `[secrets] X = {}` (missing `required`) installs without erroring.

## 2026-04-25 — [GOTCHA] Squash-merged PRs without `Closes #N` trailer leave issues orphan-open

Three v3.1 issues (#312, #314, #317) shipped to alpha in PRs that were squash-merged without `Closes #N` in the PR *body* — only in the title. GitHub's auto-close only scans the body of the squash commit message, so the issues stayed `OPEN` even though the work landed. Discovered when a sub-agent dispatch for #314 reset its worktree to current alpha and found the implementation already there. Cost: one wasted sub-agent run + an audit cycle. Fix: every PR body must include `Closes #N` on its own line (not just the title). Orchestrator pre-dispatch audit now also greps `git log --all -200` for shipped PRs that match the issue's keywords/number; codified in `.claude/iteration-cycle.md`. Also: DEV_LOG itself was not being updated for these shipped PRs — `git log` is the only reliable source for what shipped on alpha after 2026-04-11. This entry serves as a backfill anchor.

## 2026-04-25 — [DECISION] Squash-merge + install-alpha per sub-agent PR (orchestrator merges unilaterally on alpha)

User authorized the orchestrator to merge sub-agent PRs without waiting for human approval, on the condition that every merge is `gh pr merge --squash --delete-branch` so each PR lands as a single revertible commit on alpha. Bad PR → `git revert <sha>` removes it cleanly without unwinding others. Endgame: when v4 starts, alpha is not merged wholesale into v4; the squash history makes it cheap to cherry-pick the keepers. After every merge: orchestrator runs `just install-alpha` (no longer waits on user), pings user with the PR's Human verification checklist. Each new milestone branches from current alpha HEAD (cumulative).

**Breaks if:** any sub-agent PR is merged with `--merge` or `--rebase` (multiple commits per PR breaks the cherry-pick model); or orchestrator skips `install-alpha` post-merge (user can't verify against the built binary).

## 2026-04-25 — [DECISION] Alpha-train release orchestration through v3.5 (PR → alpha)

Locked in the v3.1–v3.5 roadmap (5 milestones, 26 issues) and the orchestration model that will execute it. Specs landed: `docs/specs/process/release-orchestration.md` (durable spec — alpha-train flow, per-PR `Breaks if:` + Human verification + Test added requirements, three-strike rule for sub-agents, no backwards-compat shims, batched alpha→beta promotion only on explicit signal), `docs/specs/releases/v3.1.md`–`v3.5.md` (per-release issue lists + release-level human verification checklists), `.claude/iteration-cycle.md` (operational checklist Claude reads at session start). `.gitignore` flipped from `.claude/` to `.claude/*` + `!.claude/iteration-cycle.md` so the iteration spec is tracked while the rest of `.claude/` stays local.

Rejected alternatives: per-release alpha→beta→main cycles (too much context-switching, batch promotion is cheaper); generic "next-up issue" loop without per-release human gates (verification debt accumulates and 3.5 ships unverifiable). Rejected because: alpha is the active dev branch and beta should only move when the user signals a batch promotion; verification has to be release-bounded so regressions don't compound silently across milestones.

**Breaks if:** new PRs land on alpha without `Breaks if:` lines, without a release-level human verification checklist update in `docs/specs/releases/v3.x.md`, or without a test added — the orchestration spec calls these out as hard gates.

## 2026-04-24 — [GOTCHA] `wtp add` always branches from main, not the active worktree

`wtp add -b <name>` picks up the repo's default branch (main) as the base, not the worktree you're currently sitting in. Running it from the alpha worktree still produced a branch off main — 14 commits behind. Fix: after every `wtp add`, immediately run `git log --oneline -1` in the new worktree. If it doesn't match alpha HEAD, `git reset --hard alpha` before touching any files.

## 2026-04-24 — [GOTCHA] `vertical_centered` + `horizontal_wrapped` doesn't actually center content

In egui, `horizontal_wrapped` fills the full available width and wraps left-to-right, so wrapping a it in `vertical_centered` only centers the outer widget block — the chips/labels inside still left-align. Fix: use `horizontal` (no wrap) when the content is short enough to fit on one line. The notification modal's keyboard hint row had this bug; switching to `horizontal` centered it correctly.

## 2026-04-24 — [GOTCHA] Custom Component subclasses crash if they don't inherit `Component`

`Column` calls `child._render_clipped()` on every child. Inner classes used as layout children (`_CountdownRing` in stand-up-reminder, `_Body` in quick-note/todo/wikipedia) that implement `measure/is_grow/render` but don't inherit `Component` will crash at render time with `AttributeError: object has no attribute '_render_clipped'`. Rule: any object placed inside a `Column` or `Card` must be a `Component` subclass.

## 2026-04-24 — [FIX] App code corrupted during AppBar migration — not caught until smoke test

The Header→AppBar migration in this session produced silent corruption in two apps: `screen-time`'s `_chrome_tree` had the MODE_CLOCK branch reduced to a dangling `%B %-d')}")` fragment (the subtitle f-string was deleted but a closing fragment remained), and `notification-tester`'s `AppBar(title=...)` call had extra words appended after the string close, making it a syntax error. Both were only discovered by opening the apps and seeing them crash. The Scrollable.render method was also found outside the class body (orphaned past a `return`). These are syntax-level bugs — the right safeguard is `python3 -m py_compile` on every modified app file before committing.

## 2026-04-23 — [CHANGED] Host-measured text layout primitives (issue #312, PR → alpha)
Python SDK estimated text widths with `font_size × ratio` heuristics; the host renders with real egui font metrics. This produced mis-sized badge pills, wrongly-clipped keychips, and truncation that cut at the wrong character count.

New PGAP DrawCommands: `Badge`, `KeyChip`, `KeyChipRow`, `MeasureText`; new `PlexiEvent::TextMeasured`. `DrawCommand::Text` extended with required `max_width: Option<f32>` and `elide: bool` (no serde defaults — breaking wire change). Host binary-searches for the clip point using real galleys. Deleted `_truncate_to_width`, `_char_px`, `_CHAR_W_*`, `_BADGE_*` from the Python SDK. `badge()`, `KeyRow`, `FooterKeys`, and `commit_graph.py` all rewritten to emit the new commands. `ctx.badge()`, `ctx.key_chip_row()`, `ctx.measure_text()` added to `RenderContext`.

Key decision: `MeasureText` handled inside `mod.rs`'s `ui()` drain loop (not `routing.rs`) because only `ui()` has a live `egui::Ui` for font access. `badge()` free function in `ui.py` is now void — callers needing horizontal flow advance use `_approx_badge_w()` (heuristic for cursor-only, not rendering).

**Breaks if:** Any badge pill is wrong size, or key chips show as plain text, or `ctx.text()` calls from older app code omit `max_width`/`elide` (they fail deserialization — required fields).

## 2026-04-23 — [FIX] Host PATH broken under GUI-bundle launch (alpha)
Root cause: when Plexi is launched from `/Applications` via Spotlight, Dock, Finder, or `open -a`, LaunchServices starts the process with a minimal `/usr/bin:/bin:/usr/sbin:/sbin` PATH — no Homebrew, no `~/.local/bin`, no asdf/nvm/pyenv shims. The user's shell profile never runs. Plexi then whitelists that broken PATH into every process-app subprocess via `process_app/mod.rs` (ENV_WHITELIST includes `PATH`), so apps that shell out to tools installed under `/opt/homebrew/bin` (e.g. GitHub Tree calling `gh`, any agent using `rg`/`fd`, etc.) see `shutil.which("gh") == None` and fail to auth. Running Plexi from a terminal (`plexi-alpha`) works because the shell PATH is inherited correctly.

Fix: new `shell::install_login_shell_path()` runs `$SHELL -l -c 'printf %s "$PATH"'` once at startup and `set_var("PATH", ...)` on the Plexi process itself. Falls back to prepending `/opt/homebrew/{bin,sbin}:/usr/local/{bin,sbin}` if the probe fails. Called from `main()` right after logging init, before any subprocess spawns or threads start. All downstream reads (process-app whitelist, terminal env builder, internal shelling) inherit the resolved PATH with zero per-callsite changes. Removed the duplicate homebrew-prepend hack from `shell::build_env` — single source of truth now.

**Breaks if:** Launch Plexi Alpha from `/Applications` (Spotlight/Dock, NOT from a terminal). Open a GitHub Tree pane. It says "The gh CLI isn't on PATH" or "Run `gh auth login`" despite `gh` being installed in Homebrew. Also breaks if `~/.plexi-alpha/plexi.log` shows `Login-shell PATH probe failed` without any fallback line after it.

## 2026-04-23 — [FIX] Grey square top-left of every pane — round 2, in tiling (uncommitted)
Root cause: the first grey-square fix in `src/process_app/mod.rs` only handled the `ProcessApp::ui()` path. The *outer* tile renderer in `src/tiling.rs::pane_ui` was wrapping *every* pane (app, agent, terminal) in the exact same collapsing `egui::Frame::new().fill(terminal_bg).inner_margin(8).show(ui, ...)`. Because every downstream renderer (ProcessApp, agent_pane, TerminalView) paints via `ui.painter()` / paints over its own computed rect without allocating egui UI space, the outer Frame collapsed to a tiny rect in the top-left and painted the background only there. Same for the zoomed-pane placeholder (line 97-102) — `Frame.show(ui, |_| {})` with an empty closure allocates zero.

Fix: drop the outer Frames in `pane_ui`. Paint the pane background directly with `ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, terminal_bg)`, then run the inner renderer inside a child UI built with `UiBuilder::new().max_rect(pane_rect.shrink(8.0))` to preserve the 8px inner-margin behavior. Same treatment for the zoomed-pane placeholder. `cargo build --release` clean, `just install-alpha` green.

**Breaks if:** Any pane — app, agent, or terminal — shows a small grey rectangle in its top-left corner, or the pane background/margin behavior changes (content touching the edges without an 8px gutter).

## 2026-04-23 — [FIX] Grey square top-left — round 1, in process_app (uncommitted)
Root cause: `ProcessApp::ui()` in `src/process_app/mod.rs` wrapped the draw-command playback in an `egui::Frame::new().fill(terminal_bg).show(ui, ...)`. Every primitive inside `render_draw_commands` paints via `ui.painter()`, which draws but never *allocates* UI space, so the Frame's content rect collapsed to its minimum size in the top-left and painted `terminal_bg` only over that tiny rect. Round 1 of the fix; round 2 is above (same pattern one level up in `src/tiling.rs`).

Fix: drop the Frame wrapper. Paint `terminal_bg` directly over `ui.available_rect_before_wrap()`, then call `render_draw_commands` against `ui` as before. The pane-background contract now lives in one explicit painter call against a rect we control.

**Breaks if:** An app's own background (drawn by ProcessApp before draw-commands play back) fails to fill the pane or shows a small inner grey rectangle.

## 2026-04-23 — [FIX] Modal keyboard focus leaks to app behind close-confirm (uncommitted)
Root cause: `draw_confirm_close` in `src/overlays.rs` used `ui.input(|i| i.key_pressed(Enter))` — a read-only check that does **not** consume the event. The pending-close overlay was also drawn at the *end* of `update()`, *after* `dispatch_app_key_events` had already forwarded the Enter to the focused pane. Symptom: Cmd+W on a backlog pane → confirm-modal appears → Enter both confirms the close *and* opens the selected note in the default markdown editor as the pane tears down.

Fix: migrate confirm-close onto the existing `FocusLayer` pipeline used by the notification modal. New `FocusLayer::ConfirmClose` + `sync_confirm_close_focus()` pair in `src/app/mod.rs`; `input_captured_by_overlay()` now returns true when either modal owns focus; the early-render path in `update()` dispatches to the right modal before `drain_captured_keyboard_input` clears the buffer for downstream panes. `draw_confirm_close` uses `ctx.input_mut(|i| i.consume_key(Enter/Escape))` and its late-render call site was removed. Also added a scrim and inline "Enter confirm / Esc cancel" hint for stylistic parity with the command palette and notification modal.

**Breaks if:** Cmd+W on a backlog (or any non-terminal) pane → hitting Enter on the confirm modal also triggers the selected-item action in the pane underneath; or the confirm-close modal no longer dims the background behind it.

## 2026-04-23 — [FIX] Command palette collapses to one row after a cache-miss redraw (uncommitted)
Root cause: follow-up on the earlier palette fix. `ScrollArea::max_height(...)` caps the viewport but does **not** reserve it — when the filtered entry set is short (single pane + a handful of apps), egui measured the natural content height and shrank the scroll viewport to ~1 row, even though `auto_shrink=[false, false]` was set. The earlier dynamic-height patch only addressed the cap, not the reservation.

Fix: pair `.max_height(palette_max_list_h)` with `.min_scrolled_height(palette_max_list_h)`. Both now point at the same computed target so the viewport is locked at that height regardless of content size.

**Breaks if:** Open Cmd+P with only one pane running → palette renders as a ~1-row sliver instead of a full-size list.

## 2026-04-23 — [FIX] Header sits too low in the top of the pane (uncommitted)
Root cause: `Column.padding` defaulted to `SPACE_XL` (24px) on all four sides. A `Header` at the top of a pane carries its own visual weight via `TEXT_TITLE_XL` (28pt); stacking 24px of top padding above it reads as a "dropped" title rather than an anchored one. Side/bottom padding at 24px feels correct — only the top is wrong.

Fix: add `Column.padding_top: Optional[float]` that defaults to `SPACE_LG` (16px). Sides and bottom stay at 24px. Apps that want pixel-perfect override pass `padding_top=N`. Applies to every SDK v2 app with no migration needed — the new default takes over as soon as the updated `plexi_sdk` package is installed.

**Breaks if:** `Screen Time` (or any other SDK v2 app with a `Header`) shows an obvious top gap that makes the title look like it's floating in the middle of a header region rather than anchored to the top of the pane.

## 2026-04-23 — [FIX] Apps shadowing host Cmd-chords + unclipped text overflowing columns (uncommitted)
Two systemic fixes the user flagged in the same breath.

**Cmd-chord hijack (root cause):** `ProcessApp::handle_key` in `src/process_app/mod.rs` forwarded every Key event (except bare letters) to the app subprocess as `PlexiEvent::Key`. Apps received Cmd+Enter, Cmd+P, Cmd+Shift+A, etc. and could act on them *before* the host's `poll_actions` ran in the same frame. Concrete symptom: in the backlog app, Cmd+Enter opened the selected note instead of toggling pane zoom.

Fix: skip forwarding any event where `modifiers.command` is set. Cmd-modified chords are reserved for host shortcuts; apps that want shortcuts use bare letters or Shift/Alt combos. `handle_key` still forwards non-Cmd chords (arrow keys, Enter, Esc, Tab, etc.) normally.

**Unbounded text (root cause):** `ctx.text()` in the Python SDK had no width argument. Apps drawing bounded surfaces (list rows, columns, table cells) relied on ad-hoc `line[:int(pw/7)]` truncation or nothing at all. Long note names overflowed their column in the backlog app; when the pane was narrow, every item overlapped every other item into an unreadable mess.

Fix: added `max_width: float | None = None` to `ctx.text()`. When set, the SDK truncates the string with an ellipsis *before* emitting the DrawCommand — there's no host-side clipping, so this is the only safe bound. Exposed `plexi_sdk.truncate_to_width(text, max_px, font_size, mono)` as a public helper for hand-drawn surfaces that can't route through `ctx.text`. Migrated backlog to use `max_width` on every bounded text call (header, item names, preview path + lines).

**Breaks if:** (1) In the backlog pane, Cmd+Enter opens the selected note instead of zooming the pane; or (2) long note names in backlog render past the list column's right edge into the preview pane.

## 2026-04-23 — [FIX] Command palette only rendering ~1.5 items (uncommitted)
Root cause: `src/command_palette.rs` used `ScrollArea::vertical().max_height(400.0).auto_shrink([false, true])`. With `auto_shrink_y=true` inside an `egui::Area` + `egui::Frame` with margins, the ScrollArea measures available height incorrectly on first lay-out and collapses the viewport to roughly one row. Every other ScrollArea in the codebase (agent_pane, file_browser, render/agent_pane) uses `auto_shrink([false, false])`; the palette was the outlier.

Fix: (1) switch to `auto_shrink([false, false])` to match the codebase convention — max_height alone governs the viewport. (2) Make max_height dynamic — `screen_rect.height() - 80 anchor - 120 bottom/input/padding`, clamped to a 200px floor — so the palette scales with window size instead of hard-capping at 400px.
**Breaks if:** Cmd+P opens the palette and fewer than ~6 items are visible at once, or the palette no longer grows when the window is tall.

## 2026-04-23 — [FIX] App panes were rendering monospace everywhere (uncommitted)
Root cause: `src/theme.rs:font_definitions()` inserted `JetBrainsMonoNerdFont-Light` at index 0 for **both** `FontFamily::Monospace` AND `FontFamily::Proportional`. That meant any `ctx.text(..., monospace=False)` call from the Python SDK still rendered in JetBrains Mono (a monospace font), so app UIs looked like terminal dumps even when they explicitly requested proportional.

Fix: swap priorities in the Proportional family — `DejaVuSans.ttf` (already bundled as fallback #1) now primary, `JetBrainsMonoNerdFont-Light.ttf` secondary for glyph fallback (nerd icons, box drawing). Monospace family unchanged. Apps get real proportional body text; terminal panes still use the mono font via `FontFamily::Monospace`.

Same pass also tightened SDK v2 component defaults after the user flagged the notification-tester as still feeling "dog shit" post-SDK-v2:
- `Column.padding`: SPACE_LG → SPACE_XL (24px outer padding feels airy, not cramped).
- `Column.gap`: SPACE_SM → SPACE_MD (12px between siblings).
- `Card`: added 1px border in HIGHLIGHT color — SURFACE and BG are close enough in brightness that cards weren't popping off the pane.
- `Card.padding`: SPACE_MD → SPACE_LG.
- `KeyRow.BADGE_W`: 28 → 44px, `BADGE_H`: 20 → 26px; badges now properly tile for chords (⌘K, Esc) and the glyph is monospace-centered.
- `Header`: title sized to TEXT_TITLE_XL (28pt, was 20pt); clearer title-to-subtitle-to-divider spacing; baseline math fixed so subtitle sits below the title instead of overlapping.
- `Footer`: symmetric top/bottom breathing room around the divider — previously hugged the pane frame edge.

**Breaks if:** UI pane text renders monospace again. Card borders invisible (border=HIGHLIGHT removed). KeyRow badges rendered smaller than the description text height. Header subtitle overlaps title baseline. Footer text crowds the pane bottom edge. Fallback Unicode coverage (nerd icons) breaks because JetBrainsMono fell out of the Proportional fallback chain entirely (it should still be at index 1).

## 2026-04-23 — [DECISION] SDK v2 — declarative UI primitives + `ui-playground` (uncommitted)
Every app before today used `ctx.rect`, `ctx.text`, `ctx.circle` with hand-picked pixel coordinates. Result: every new app reinvented header layout, padding, truncation, footer placement — and the notification-tester screenshot made it concrete (footer clipping on the right edge, no padding in buttons, subtitle cut off).

Added `sdk/python/plexi_sdk/ui.py` — a declarative component tree. Components are dataclasses (`Column`, `Card`, `Header`, `Section`, `Divider`, `Heading`, `Label`, `KeyRow`, `ScrollLog`, `Spacer`, `Footer`). Each has `measure(avail_w) -> height` and `render(ctx, x, y, w, h) -> None`. `Column` lays children top-to-bottom; `Spacer(grow=True)` soaks slack; `Label` wraps up to 3 lines; `Footer` wraps up to 2; `Heading` and `KeyRow` truncate with `…`. `ctx.render(tree)` clears the pane and lays out the tree.

Style tokens are re-exported from ui.py and mirror `src/style.rs` (`SPACE_*`, `TEXT_*`, `RADIUS_*`, palette constants). Low-level `ctx.rect` / `ctx.text` remain — they're the escape hatch, not the starting point.

Install recipes (`install-alpha`, `install-beta`, `install-v3`) now copy the entire `plexi_sdk/` package directory to `~/.plexi-*/sdk/plexi_sdk/` instead of a flat `plexi_sdk.py`, so `from plexi_sdk.ui import ...` resolves. Rejected: keeping the flat-file layout and concatenating ui.py contents into `__init__.py` — cohesion lost at 1000+ LOC in a single file.

Also landed: `examples/ui-playground/` — a reference app that renders every component at once, plus `docs/sdk-ui-guide.md` with the component table, responsive behavior notes, and the "write your own component" recipe. Notification-tester migrated from raw primitives to SDK v2 as the proof point; new code is ~40% shorter and declarative. Smoke test passes — both the tester and playground boot cleanly with the new package layout.

**Breaks if:** `from plexi_sdk.ui import Column` fails at app startup (the installed SDK is still flat-file; run `just install-alpha`). Notification-tester footer clips on a narrow pane (Footer wrap broken). Scroll log shows oldest lines at the top (should be newest-first). Card background doesn't honor the `radius` token. Heading text doesn't truncate with `…` when pane is narrower than the title. Install leaves a stale `~/.plexi-*/sdk/plexi_sdk.py` file alongside the new package dir (rm in the recipe missed).

## 2026-04-23 — [FIX] install-alpha / install-beta refresh macOS LaunchServices (uncommitted)
`install-beta` was only renaming the `.app` bundle but not refreshing macOS LaunchServices — so the Apple menu / app menu bar kept showing the cached "Plexi" name instead of "Plexi Beta". `install-v3` already had the `lsregister -f` + `pbs -update` calls; ported them to `install-alpha` and `install-beta` for consistency.
**Breaks if:** a fresh `just install-beta` leaves the menu bar showing anything other than "Plexi Beta" next to the Apple menu.

## 2026-04-23 — [CHANGED] Input-kind notifications are multiline with Cmd+Enter submit (uncommitted)
Input-kind notifications used `TextEdit::singleline` + Enter-to-submit — single short text only. Use cases the user wants (describe what you're working on, write a quick note) want multi-line. Rejected: embedding a full text editor (way out of scope for a notification). Chose `TextEdit::multiline` + `Cmd+Enter` submit — same chord Slack / Linear / Discord use. Enter inserts a newline into the buffer; `Cmd+Enter` commits. Input field now has `desired_rows(6)` so it's obviously multi-line on first render and scrolls vertically once content exceeds the visible row count.

`Cmd+Enter` is consumed via `ctx.input_mut.consume_key` inside `draw_notification_modal` so egui's `TextEdit` can't see it and can't invent a widget-local interpretation. Footer hint for input kind reads "Enter for newline · ⌘⏎ to submit · Esc to dismiss" (required variant drops Esc).

**Breaks if:** pressing Enter in the input field submits instead of inserting a newline. Cmd+Enter inserts a newline and doesn't submit. Input field renders as a single-line edit. Input field doesn't scroll when content exceeds 6 rows. Submitted value returns without the newlines the user typed (trim() only strips leading/trailing whitespace — internal newlines must survive).

## 2026-04-23 — [DECISION] `src/style.rs` design tokens + notification modal polish (uncommitted)
Added a central design-tokens module so every overlay/pane references the same spacing, typography, radii, modal widths, and button heights. The set is intentionally minimal — only tokens referenced by the current codebase are exported; scale holes (`SPACE_XS`, `MODAL_WIDTH_SM`, `RADIUS_SM`, etc.) are added as migrations need them, not speculatively (project rule against `#[allow(dead_code)]` and speculative abstractions).

Notification modal polished on top of the new tokens:
- Width 520 → 640 (`MODAL_WIDTH_MD`), inner padding 32h / 28v.
- Title centered and bumped from 20pt → 28pt (`TEXT_TITLE_XL`).
- Body centered, max-width clamped so long lines wrap naturally.
- Options rewritten as a hand-drawn `option_button` widget in `overlays.rs` — egui's built-in `Button` left-aligns labels and gives no API hook to center them inside a fixed-width rect; painting manually gets us a centered label + right-gutter shortcut hint (`[Y]`) + 52px tall button (`BUTTON_H_LG`). Focused option gets the accent fill + black text; hover on non-focused options lifts the bg slightly. Screenshot-reproducible: the previous "Yes  [Y]" with zero horizontal padding is gone.
- Message-kind `Acknowledge` is now a fixed-width `primary_button` — 220px wide, centered on its own row, 40px tall.
- Footer hint centered below the button instead of sharing a row with it.
- Scrim alpha 170 → 190 (slightly deeper dim).

**Breaks if:** option buttons render with left-aligned text. Focused option in a choice notif doesn't visually pop (no accent fill). Notification title is left-aligned, not centered. Modal width feels cramped (anything less than ~600px suggests the token wasn't applied). Acknowledge button for message kind spans the full modal width instead of being centered at 220px. Hover on a non-focused option doesn't visibly lift. `cargo build` emits a warning about unused constants in `style.rs` (the minimal-tokens rule was broken).

## 2026-04-23 — [DECISION] FocusLayer input-capture primitive (uncommitted)
Addresses a recurring class of bug: keystrokes leaked through overlays to panes behind them. Root cause — egui doesn't auto-route input by visual stacking; any widget that reads `ctx.input(...)` sees the same event stream. Every new overlay had to independently remember to gate input, and every miss reintroduced the same leak.

Introduced `FocusLayer` enum + `focus_stack: Vec<FocusLayer>` on `PlexiApp` with semantics: the top-of-stack layer owns keyboard input for the frame. When a non-`Pane` layer is on top, `drain_captured_keyboard_input` retains only a global allowlist (`Cmd+Q`, `Cmd+W`, `Cmd+Shift+A`, `Cmd+]`/`Cmd+[`) in `ctx.input.events`, dropping every other `Event::Key` and all `Event::Text`. Mouse events pass through untouched.

Integration: at the top of `update()`, `sync_notification_modal_focus()` reconciles the layer against `show_notification_modal && !pending_notifications.is_empty()`. If `input_captured_by_overlay()` returns true, the modal renders FIRST (so its `TextEdit` for the `input` kind consumes typed chars), then the drain runs. `dispatch_app_key_events` is gated on the same predicate — focused apps don't receive `handle_key` when an overlay owns input. `keys::poll_actions` runs afterward over the drained buffer, so global keybinds (and Cmd+]/Cmd+[ queue-cycling) still work. The late-in-`update()` modal render is gone — only the early phase renders now.

Only the notification modal is migrated. Command palette, run palette, rename pane, confirm close, quit confirm, shortcuts overlay, and secrets manager still use their legacy per-handler input paths; the migration recipe is filed in `~/.plexi-alpha/backlog/note-2026-04-23-migrate-overlays-to-focus-stack.md`. `FocusLayer` has one variant today (`NotificationModal`) — extend as each overlay migrates.

Rejected: intercepting input in the draw function alone (that's what we had — doesn't stop panes from reading the same events). Rejected: a full "only top layer can read input" enforcement via egui's focus API (the terminal backend doesn't use egui focus; it polls events directly, so only a buffer drain works).

**Breaks if:** typing into the notification-tester's `input` kind doesn't reach the modal's input buffer. Opening a choice notification while a terminal pane is focused — keys like `j` or `k` — causes the terminal to receive them too. Cmd+Q doesn't quit while the notification modal is open. Cmd+Shift+A doesn't close the modal while a required notif is up. Cmd+]/Cmd+[ doesn't cycle the queue. Focused app still receives `handle_key` while the modal is open (check `dispatch_app_key_events` gating). `focus_stack` leaks layers across modal open/close cycles (grows unboundedly).

## 2026-04-23 — [CHANGED] Notifications: multi-kind keyboard-first modal, [notifications] config, notification-tester app (uncommitted)
Second-pass redesign of the notification system on top of the earlier modal pass. Key shifts:

- **Kinds.** `DrawCommand::Notify` grows a `kind: NotifyKind` (`message` / `choice` / `input`) plus `options: Vec<NotifyOption>`, `input_prompt`, `required`. Back-compat: missing `kind` deserializes to `Message`, so existing apps (stand-up-reminder, etc.) keep working unchanged. `NotifyOption { label, value, shortcut }` is the choice-button model; `NotifyKind::Input` uses `input_prompt` as the placeholder hint.
- **PlexiEvent::NotifyAction.value.** New optional field on the event delivered back to the app. For choice kind it carries the option value; for input kind the typed text; absent for plain acknowledge/cancel. Legacy `action_label` still identifies which button/path fired.
- **Modal rewrite.** `draw_notification_modal` is now keyboard-first:
    - `Enter` / `Space`: confirm (acknowledge for message, focused option for choice, submit for input).
    - `↑↓` / `j/k`: cycle choice options.
    - `1-9`: direct-select the Nth option.
    - per-option `shortcut` char: direct-select that option.
    - `Esc`: cancel (only when `required == false`).
  The scrim catches clicks so they don't leak to panes behind. Modal Area lives on `Order::Tooltip` above a separate `Order::Foreground` scrim Area.
- **Sidebar panel deleted.** `draw_notification_panel`, `show_notification_panel`, and `Action::ToggleNotificationPanel` are gone — the modal + queue cycle supersedes it. `Cmd+Shift+A` now maps to `Action::ToggleNotificationModal`. `Cmd+]` / `Cmd+[` cycle the queue when the modal is open (the keybind is context-sensitive: tab-cycling otherwise, queue-cycling when modal is up). `modal_queue_offset` is the currently-viewed index; acknowledging pops at offset, cycling moves without acknowledging.
- **`[notifications]` config section.** `enabled` (master switch; false silently drops at both `ShowNotification` and `notify.sock` intake paths) and `focus_mode` (when true, arrivals queue silently and the user opts into review with Cmd+Shift+A). Both cached onto `PlexiApp` at startup from the config; no hot-reload. Alpha's config.toml was rewritten in the fat beta-style with full inline docs; beta's config gained the `[notifications]` block.
- **`actions` field dropped from the ShowNotification path.** `DrawCommand::Notify.actions` was being ambiguously used for both (a) server-side side effects (`resume_run` / `open_intent` / `run_command`, handled in `process_app/routing.rs` before the UI ever sees it) and (b) UI buttons. New `options` + `kind = choice` handles the UI case cleanly; `actions` remains in the protocol for side effects only and is explicitly dropped after side-effect processing (`let _ = actions;` in routing.rs).
- **Python SDK.** `ctx.notify_choice(title, options, required)` and `ctx.notify_input(title, prompt, required)` added; both block for a response. `ctx.notify(...)` and `ctx.notify_and_wait(...)` unchanged. `notify_action` event dispatch in the main loop now returns the `value` when present, `"__cancel__"` for Esc-cancels.
- **notification-tester example app.** New playground at `examples/notification-tester/`. Keys: `m` / `c` / `i` fire each kind; `b` queues a 3-message burst so you can exercise `Cmd+]`/`Cmd+[` cycling; `r` fires a required (Esc-resistant) message. On-pane log shows what came back.

Rejected: inferring `kind` from field presence (breaks on future media kinds, ambiguous when both `options` and `input_prompt` are set). Rejected: `type` over `kind` (Python reserved word, keyword-conflict in SDK). Deferred: media kinds (image/audio/video/rich), snooze, centralized history view, visual composition canvas + Moss transforms.

**Breaks if:** stand-up-reminder fires and no modal appears. Cmd+Shift+A no longer opens any notification surface. Sending `{"type":"notify"}` without a `kind` field stops working (back-compat gone). Cmd+] in a normal terminal stops cycling tabs (the modal_open gate is inverted). Notification-tester `c` key shows options but Enter doesn't pick the focused one. Choice option values aren't delivered as `NotifyAction.value`. Setting `[notifications].enabled = false` doesn't silence notifications. Setting `focus_mode = true` still auto-pops the modal on arrival.

## 2026-04-23 — [CHANGED] Notifications: centered modal as primary surface (uncommitted)
The in-app `ShowNotification` path never set `show_notification_panel = true` (only the `notify.sock` drain path did), so `ctx.notify(...)` calls from apps like `stand-up-reminder` silently accumulated in `pending_notifications` with the badge incrementing but no visible window. Fixed the surfacing by introducing a new `show_notification_modal` state that auto-opens a centered work-area modal on every `ShowNotification` arrival (both in-process apps and socket-posted notifications). Modal dims the background (Area at `Order::Foreground` with an alpha-170 scrim, then a second Area at `Order::Tooltip` centered via `Align2::CENTER_CENTER`), shows one notification at a time (front of the queue), and auto-closes once the queue drains. Escape and the default "Acknowledge" button both send `NotifyAction { action_label: "acknowledge" }`. Sidebar panel (Cmd+Shift+A) kept as secondary history view; its fallback button also renamed from "Dismiss" to "Acknowledge" for consistency. Rejected: a display-wide OS-level takeover (too brutal for every reminder, bigger primitive to build — escalate later if a real use case demands it). Centralized notification management (history/snooze/mute-per-app) filed in `~/.plexi-alpha/backlog/note-2026-04-23-centralized-notifications.md` as a separate feature.
**Breaks if:** Stand-up reminder fires on schedule but no centered modal appears (the bell badge still increments but the primary surface is broken). Acknowledge button doesn't pop the notification off the queue. Modal stays up after the queue is empty. Cmd+Shift+A sidebar panel no longer opens at all (regression — it should still toggle as the history view).

## 2026-04-22 — [CHANGED] Configurable Cmd+Q and Cmd+W confirmation dialogs (PR → alpha)
Added `confirm_quit` and `confirm_close` top-level fields to `PlexiConfig` (both default `true`). `confirm_quit` supersedes the old `[beta].quit_confirm` flag (falls back to beta for backwards compat). `confirm_close` gates a new modal dialog shown before closing a pane via Cmd+W — `execute_close_pane()` in `pane_ops.rs` consolidates the close logic so both the immediate and dialog-confirm paths share one implementation. `draw_confirm_close` in `overlays.rs` patterns exactly after `draw_quit_confirm_overlay`. When `confirm_close = false`, closes happen immediately with no dialog.
**Breaks if:** Cmd+W closes a pane without showing the dialog when `confirm_close` is unset/true. `confirm_quit = false` in config.toml still requires triple-press to quit. `execute_close_pane` on the last pane of the last context deletes the context instead of resetting to a blank pane.

## 2026-04-20 — [CHANGED] lava-opus app shipped (examples/lava-opus/)
Buoyancy-driven blob simulation with fake-metaball blending via `DrawCommand::Circle` layering. Named "lava-opus" (not "lava-lamp") to avoid collision with a second lava app in flight. Physics: temperature-driven rise/fall, viscous drag, wall repulsion, soft bounce. Render: glow halo + solid core + specular highlight per blob; translucent bridge circles between nearby pairs fake the metaball merge look. Click to inject heat. Installed and verified at `~/.plexi-v3/apps/lava-opus/`.
**Breaks if:** app registry fails to load `lava-opus` (id mismatch in manifest.toml — it must be `id = "lava-opus"`). Blobs teleport to top/bottom edge (bounds clamp regressed). Bridge circles disappear entirely between nearby blobs (MERGE_FACTOR or bridge alpha logic changed).

## 2026-04-20 — [DECISION] Where Were We snapshot
Fresh session orientation. Last committed work: dead code sweep (PlexiIQ removal, zero warnings). v3 is in clean state — only `Cargo.lock` modified in worktree. Open work: `V3_STEP_9_FOLLOWUPS.md` tracks remaining brokers (PipeSend peer routing, RunUpdate round-trip, media handlers, FD CLOEXEC audit) before v3.0.0 tag.
**Progress:** All 12 refactor steps done; HTTP broker live; CI gate real; 74+ tests green.
**Open:** Step-9 follow-ups (see `V3_STEP_9_FOLLOWUPS.md`), then tag v3.0.0.

## 2026-04-20 — [CHANGED] Dead code sweep: PlexiIQ removal + zero-warning cleanup (→ v3)
Removed PlexiIQ agent feature entirely from v3 (preserved on `feature/plexi-iq` branch at d3c4c1f). Removed `Pane::Agent`, `AgentPane`, `TurnMsg`, `spawn_agent_pane`, agent workspace restore, `Action::SpawnAgentPane`, and the agent rendering block in `tiling.rs` (~175 LOC). Removed 8 dead `HostCommand` variants, 7 dead `HostEffect` variants, `PermissionsLog`/`PermissionDecision` structs (permissions.jsonl persistence never wired), `AppReply` enum (superseded by `DrawCommand::Ready`), `FsService`/`SecretsService`/`SpawnService` trait objects (zero production readers after STEP-5 rework), and all module-level `#[allow(dead_code)]` from `main.rs`. 1,311 lines deleted, 0 warnings, 0 errors.
**Breaks if:** `cargo build` produces any warning (dead_code allows are gone — they were the only gate). PlexiIQ agent pane survives on v3 (it shouldn't — feature/plexi-iq is the preservation branch).

## 2026-04-19 — [CHANGED] File explorer + protocol fixes (→ v3)
Three fixes landed together. (1) **`DrawCommand::Ready`**: the SDK's post-Init handshake message `{"type":"ready",...}` was not a recognized `DrawCommand` variant — the host background reader logged a WARN and discarded it every launch. Added `Ready { sdk, features_used }` to the enum; `process_app/mod.rs` now stores `sdk`/`features_used` on `ProcessApp` and the render.rs no-op arm covers it. FrameDone mismatch log demoted from WARN to DEBUG (1-frame async lag is expected behavior, the spam was masking real issues). (2) **HTTP non-blocking**: `routing.rs::HttpRequest` was calling `self.net.http(...)` synchronously on the egui UI thread — froze the entire host for the duration of every Wikipedia search. Fixed by spawning a per-request background thread that writes its result to a `mpsc::Sender<PlexiEvent>`; `ui()` drains the receiver before flushing outbound events. (3) **File explorer app** (`examples/file-explorer/`): new PGAP app that joins the `cwd` group (receives `PathChanged` when any linked terminal cd's), lists the current directory with dirs first, and navigates with arrow keys / Enter / Backspace. (4) **`"above"` layout side**: new manifest schema value `layout_hint = { side = "above", split = 0.75 }` puts the new pane at the TOP of a horizontal split — file explorer uses this so it appears above the terminal rather than below it. `split_with_new_pane` gains a `new_pane_first: bool` parameter; `open_pane_layout` returns a 4-tuple.
**Breaks if:** wikipedia still shows "malformed draw command: unknown variant `ready`" in the log (Fix 1 regressed). A wikipedia search freezes the UI for >1s (Fix 2 regressed — HTTP back on UI thread). File explorer opens below the terminal instead of above (Fix 4 regressed). `cargo test` fails (`Ready` variant unhandled in an exhaustive match somewhere).

## 2026-04-19 — [CHANGED] V3 step 9a: real HTTP broker via ureq (→ v3)
Replaced `StubNetService` with a pure-Rust blocking `UreqNetService` (ureq 2.12 + tls). `NetService` trait extended with `fn http(method, url, headers, body) -> HttpResponse`; `http_get` becomes a default method wrapping the new signature. `ProcessApp` now holds an `Arc<dyn NetService>` clone; `routing.rs::HttpRequest` issues the real call and pushes `PlexiEvent::HttpResponse { request_id, status, body, error }` — capability-denied path still returns 403 without hitting the network. Test harness ditched its custom `http_mocks` dict + `mock_http` method in favor of `h.set_net(Arc::new(MockNetService::with(url, body)))` — the same seam production panes use. Acceptance: new Layer-1 test `layer1_wikipedia_http_broker_end_to_end` spawns wikipedia, injects a `MockNetService`, types R/u/s/t/Enter, and asserts "Rust (programming language)" surfaces in rendered output via the real broker pathway. Wikipedia example migrated from `urllib.request.urlopen` to `self.emit.http_get` so it actually exercises the broker. Along the way: added `tiny-skia` and `fontdue` to `Cargo.toml` (step 1 added `src/headless_renderer.rs` using those imports but never declared the deps, leaving `v3` HEAD un-buildable); deleted stale per-example `plexi_sdk.py` copies and pointed the Layer-1 harness at the canonical `sdk/python` via `PYTHONPATH` so examples pick up current SDK features (`on_inject`, `http_get`); added a `sink_opened` startup heartbeat to `FileEventSink::new` so the post-install smoke test's `effects.jsonl non-empty` gate actually passes; smoke test now clears `PLEXI_RUNNING=` before launching the host so running `just install-v3` from inside a Plexi terminal no longer short-circuits.
**Breaks if:** wikipedia search hangs with a spinner when hitting a real URL (broker not wired). A Layer-1 test uses `h.mock_http(..)` after this lands (method is gone — migrate to `h.set_net(Arc::new(MockNetService::new().with(..)))`). `net.http` capability denial returns anything other than 403 on `PlexiEvent::HttpResponse`. `effects.jsonl` is empty after `just install-v3` (regression in either the sink_opened heartbeat or the smoke test unsetting PLEXI_RUNNING).

## 2026-04-18 — [FUTURE] Step-9 broker follow-ups plan committed (V3_STEP_9_FOLLOWUPS.md, 13e1035)
`V3_STEP_9_FOLLOWUPS.md` at the worktree root enumerates the 5 items deferred from the scoped-down step 9 (HTTP broker, PipeSend peer routing, RunUpdate round-trip, media brokers, FD CLOEXEC audit). Each has file paths, acceptance test, `Breaks if:`, effort estimate, and order hints. Deferred because (a) each is independently mergeable and (b) step 9's 6-hour estimate in `V3_REFACTOR_PLAN.md` was too much for one session on top of the other 11 steps. Start a fresh session with 9a (HTTP broker — lowest risk, easiest ship); 9a + 9e can parallelize. After all ship, tag v3.0.0 and delete the file.

## 2026-04-18 — [CHANGED] V3 refactor step 12: invariant enforcement (→ v3)
Core invariants now have tests or structural guards. I-1 (HostModel zero egui): new `invariant_i1_host_module_is_egui_free` test reads every `.rs` under `src/host/` at test time and asserts no `use egui::` / `use eframe::` line — comments mentioning egui are fine. I-5 (Pane ADT frozen at 3 variants): `PaneRuntimeKind` now carries a doc comment pointing at `docs/specs/releases/plexi-v3.0.md §2` noting the freeze; changing requires a spec amendment. I-10 (capability grants per-workspace): guard test exercises the `MockSecretsService` lookup contract — production triple filtering lives in `app_permissions::PermissionsLog::check`, which was already tested via TryFrom in STEP-2. I-2 (no `todo!()`/`unimplemented!()` outside tests): already enforced by `#![deny(clippy::todo, clippy::unimplemented)]` in `main.rs`. Full test matrix: 74/74 passing, release build clean. Step-9 follow-ups (real HTTP broker, PipeSend peer routing, media brokers, CLOEXEC audit) remain the last known scope for a separate session before v3.0 tags.

**Breaks if:** `use egui::` appears in `src/host/*.rs` without triggering a test failure. A fourth variant lands in `PaneRuntimeKind` without the spec-amendment doc being updated.

## 2026-04-18 — [CHANGED] V3 refactor step 11: CI gate that actually enforces (→ v3)
`.github/workflows/plexi-v3-test.yml` replaces the vacuous `cargo test pgap_test_harness` step with a real matrix: `cargo test --release` (all 72 tests including host + harness + Layer-1), `uv sync --all-groups && uv run pytest -q` (SDK widget + example Python tests), `scripts/smoke-test.sh` (host launch + effects.jsonl growth check). `uv` is bootstrapped via the official astral installer. `scripts/smoke-test.sh` grows a new assertion: `effects.jsonl` must be non-empty after launch — catches a FileEventSink regression that the old "no panic in log" check would miss. `justfile::install-v3` now actually runs `lsregister -f` + `pbs -update`; CLAUDE.md's claim is no longer a lie.

**Breaks if:** CI is green while `cargo test --release` fails, `uv run pytest` fails, the smoke test fails, or `effects.jsonl` stays empty. Right-click → Services doesn't show Plexi v3 after `just install-v3`.

## 2026-04-18 — [CHANGED] V3 refactor step 10: real Rust Layer-1 tests + uv runner (→ v3)
`pgap_test_harness` grows from zero `#[test]` fns to five: init/ready handshake, render + frame_done round-trip, shutdown lifecycle, todo `path_changed` cwd update, wikipedia inject-state render. Tests auto-skip when `python3` is not on PATH so local dev without Python doesn't fail — CI should fail when a real gate is missing. `pyproject.toml` at repo root: `requires-python = ">=3.11"`, `pytest` as a dev dep, `testpaths` covering `sdk/python/tests` + every example's `tests/` dir, `pythonpath = ["sdk/python"]` so `from plexi_sdk import ...` resolves under `uv run pytest`. 72/72 Rust tests green.

**Breaks if:** `cargo test layer1` returns zero tests (STEP-10 regressed). `uv sync && uv run pytest` can't resolve the `plexi_sdk` import. A ci job runs `cargo test pgap_test_harness` and matches zero tests (old shape).

## 2026-04-18 — [CHANGED] V3 refactor step 9: PGAP surface — env isolation + bold + AppSpawned SDK hook (→ v3)
Scoped-down step 9 — landed the three high-leverage items, explicitly deferred the rest. (1) **Env isolation** (spec I-6): `ProcessApp::launch` now calls `.env_clear()` and whitelists `HOME`/`PATH`/`LANG`/`LC_ALL`/`TERM`/`USER`/`SHELL` plus every `PLEXI_*` var. `ANTHROPIC_API_KEY` and similar host credentials can no longer leak to subprocess apps. (2) **Bold text rendering**: `process_app/render.rs` stops destructuring `bold: _` and routes it into an `egui::FontFamily::Name("bold")` that painter falls back to Proportional on if not registered — bold is now readable from app code without breaking rendering. (3) **AppSpawned SDK handler**: `sdk/python/plexi_sdk/__init__.py` adds `elif t == "app_spawned"` and calls `on_app_spawned(pane_id, type_id)`; default is no-op.

**Deferred to a follow-up PR** (all flagged `// STEP-9 follow-up` in source): real HTTP broker (routing.rs still logs + returns a stub HttpResponse; MockNetService seam is live in HostServices), PipeSend peer routing (TODO already in routing.rs), RunUpdate round-trip on RunComplete, Image/Video/Audio broker plumbing, `O_CLOEXEC` audit on UnixListener FDs. Scope control — shipping correctness on env isolation and the SDK handshake this pass, leaving the broker work for a dedicated session.

**Breaks if:** a spawned app can read `ANTHROPIC_API_KEY` / any host credential from `os.environ`. A SpawnApp confirmation round-trip doesn't fire `on_app_spawned` in the spawning app. `PLEXI_AUDIO=mock://` stops passing through to apps (whitelist regression).

## 2026-04-18 — [CHANGED] V3 refactor step 8: manifest schema freeze (→ v3)
`manifest.toml` gains a dedicated `[launch]` section that replaces the launch-time fields previously squatting in `[app.capabilities]`. New schema: `[launch].join_group` (was `[app.capabilities].group`), `[launch].layout_hint = { side, split }` (was `[app.capabilities].layout_hint: Option<String>` + `[app.capabilities].initial_share: Option<f32>`), `[launch].keyboard_capture` (was `[app.capabilities].keyboard_capture`). `side` must be `"right"` / `"below"` / `"overlay"`; `split` must be in (0.0, 1.0). Install-time validator fails loudly on bad values. `keybinding` field dropped — re-add when a global shortcut registrar actually consumes it. Migrated 5 example manifests (audio-recorder, quick-note, snake, todo, wikipedia) + Python and Rust scaffolder templates. Three new schema tests.

**Breaks if:** installing a v2 manifest with `[app.capabilities].group = "cwd"` silently loses the grouping (old field now ignored). `plexi-v3 app new foo` produces a manifest that fails the install-time validator. `layout_hint.side = "bogus"` installs without error.

## 2026-04-18 — [CHANGED] V3 refactor step 7: capability enforcement complete (→ v3)
Install-time validation in `AppRegistry::load_app`: a manifest whose `capabilities` list contains any unknown string fails loudly with a named error — the silent `From<&str> → FsRead` fallback removed in STEP-2 is now reinforced at the install boundary. `parse_capability_strings` (STEP-2) is the single predicate. Runtime checks in `process_app::routing`: `PipeSend` now requires `pipe.open`; `HttpRequest` requires `net.http` (and returns 403 `HttpResponse` when denied so apps see a clean failure); `AudioPlay` requires `audio.playback`; `AudioCapture` requires `audio.record`. Deleted dead `check_cd` + `path_within_scope` (Cd is not a spec DrawCommand; re-add when one exists). Two new tests: `app_registry_rejects_unknown_capability_in_manifest` and `app_registry_accepts_all_nine_spec_capabilities`.

**Breaks if:** installing an app with a typo'd capability (e.g. `"net.http_"`) succeeds without a log error. An app that didn't declare `net.http` can call `ctx.http_get()` and get back data. `PipeSend` works from an app that never declared `pipe.open`.

## 2026-04-18 — [CHANGED] V3 refactor step 6: FileEventSink wired as production event bus (→ v3)
Every `HostEffect` is now durable. `FileEventSink` opens `<config_dir>/effects.jsonl` in append mode at startup and writes one JSONL line per effect. `HostServices::new()` installs it as the production `event_sink` (was `NoopEventSink`). `HostEffect` + `HostEvent` + `PaneRuntimeKind` + `Placement` + `ShareRatio` all derive `Serialize`. Uses a separate file from `events.jsonl` (app-event bus via `crate::event_log`) to keep the host-state stream distinct from app-initiated events. FULL consumer rewiring (navigate/close driven by `FocusChanged`/`PaneClosed` effects instead of `PlexiApp`'s geometric search) was deferred — the observation layer is now durable; the consumer side ships with STEP-9 once pane-ID reconciliation has one more integration pass.

**Breaks if:** `<config_dir>/effects.jsonl` stops growing when user actions fire. `FileEventSink::new()` panics on IO error instead of logging + falling back to no-op. Re-opening Plexi discards the effects file (not append mode).

## 2026-04-18 — [CHANGED] V3 refactor step 5: HostServices gains fs/secrets/net/spawn seams (→ v3)
`HostServices` grows from 1 field (`event_sink`) to 5: `fs` (`RealFsService` / `MockFsService`), `secrets` (`KeychainSecretsService` wrapping `crate::secrets::get_secret_scoped` / `MockSecretsService`), `net` (`StubNetService` returning 501 until STEP-9 ships the real broker / `MockNetService`), `spawn` (`LoggingSpawnService` / `MockSpawnService`). `HostServices::new()` wires production; `HostServices::mock()` wires in-memory fakes for Layer-2 tests. NOT yet wired: production `ProcessApp::routing::SecretGet` still calls `crate::secrets::get_secret_scoped` directly — STEP-9 will pipe it through `services.secrets` when the broker threading is done. Three new tests lock the mock behavior.

**Breaks if:** a Layer-2 test that constructs `HostServices::mock()` still hits a real keychain/network call; `cargo test` depends on network connectivity; the production `event_sink` regresses from `NoopEventSink` before STEP-6 lands.

## 2026-04-18 — [CHANGED] V3 refactor step 4: pane-ID reconciliation (→ v3)
`HostModel` is now the sole pane-ID allocator. `PlexiApp::next_pane_id` field deleted; every `new_id = self.next_pane_id; self.next_pane_id += 1;` site in `pane_ops.rs` (8 sites) routes through either (a) consuming the returned `PaneOpened.pane_id` / `SplitOpened.pane_id` from the effect or (b) calling `HostModel::alloc_pane_id()` directly (for paths like `create_single_pane_tree`, `new_tab`, `spawn_agent_pane` that don't submit a `HostCommand`). `open_pane_layout` now returns `(PaneId, ShareRatio, bool)`. Workspace restore seeds `HostModel::next_pane_id` via `seed_next_pane_id(..)`; workspace save persists `host.next_pane_id()`. Two new tests: `ids_synchronize_across_commands` (3 OpenPane + 2 SplitVertical → every effect ID lives in `ctx.panes`) and `seed_next_pane_id_resumes_allocator`.

**Breaks if:** restarting Plexi with a saved workspace re-allocates pane IDs from 1 instead of resuming past the saved high-water mark. `egui_tiles::Tile::Pane(pid)` references a `pid` that doesn't exist in `HostModel::ctx().panes`. A new `OpenPane` returns `PaneOpened { pane_id: N }` but `ctx.panes.insert(M, ...)` with `M != N` (the old double-alloc bug).

## 2026-04-18 — [CHANGED] V3 refactor step 3: finish or delete stubs (→ v3)
Delete `src/plexi_iq/prompt.rs` (Stage-0 tombstone — v3.0 IQ operates without a templated system prompt; re-add when the turn loop actually needs one). Delete `src/plexi_iq/tools/mod.rs` (empty `ToolRegistry` with zero registrations — re-add when a real tool lands). Simplify `src/plexi_iq/context.rs` to the 2 fields the backends actually read (`pane_id`, `directory_scope`); replace its vestigial `PaneId(pub u64)` newtype with the canonical `tiling::PaneId` alias established in step 2. Remove `examples/video-player/` from the ship set — depends on a host video broker that step 9 may not land in the v3.0 window.

**Breaks if:** any module imports `crate::plexi_iq::prompt` / `tools` (step would not compile) or the IQ pane fails to spawn because `PlexiIqInstance` lost a field it actually used.

## 2026-04-18 — [CHANGED] V3 refactor step 2: unify dual types (→ v3)
One canonical representation per concept: `keys::Direction` re-exported from `host::command`, `tiling::PaneId = u64` alias kept, `app_permissions::Capability` extended with `AudioRecord`/`AudioPlayback`/`VideoPlayback` (9 spec caps), `app_protocol::PlexiEvent` gains `InjectState` + `HttpResponse`, `app_protocol::DrawCommand` gains `HttpRequest` + `Image` + `VideoPlayer` + `AudioMeter` + `AudioPlay` + `AudioCapture`. Silent `From<&str> → FsRead` fallback replaced with `TryFrom<&str>` that returns `UnknownCapability`; callers log + drop/deny instead of surfacing as an inert `FsRead`. Added `parse_capability_strings(...)` for manifest loaders (step 7/8 consume). 4 new tests lock the roundtrip + rejection behavior.

**Breaks if:** a manifest with `capabilities = ["bogus"]` silently maps to `FsRead` instead of logging a warning. Any `PlexiEvent::InjectState` or `DrawCommand::HttpRequest` wire payload fails to deserialize.

## 2026-04-18 — [CHANGED] V3 refactor step 1: dead-code sweep (→ v3)
Deleted `src/protocol/{effect,event,output,schema}.rs`, `src/input/` (entire), `src/error.rs`, `src/media/mod.rs` — 582 LOC of tombstone modules with zero external callers. Only `protocol::view` kept (used by `HeadlessRenderer`). Scaffolder templates in `src/cli.rs` migrated from v2 capability names (`terminal_write`, `filesystem = "read_only"`) to v3 `capabilities = ["fs.read"]`. Module-level `#[allow(dead_code)]` scrubbed from `src/main.rs`: 16 → 10, with each remaining one annotated `// STEP-N: <reason>` so future steps know which ones they unlock. 54/54 tests green, zero warnings.

**Breaks if:** `cargo build` re-introduces warnings (dead-code sweep regressed) or `plexi-v3 app new foo` emits a manifest whose capabilities fail validation.

## 2026-04-18 — [CHANGED] Cutover slices 2/3/4: split_focused + close_focused + navigate route through HostModel (e444d04, f601842 → v3)
Slice 2 (`split_focused`) consumes the returned `SplitOpened.placement` to derive the split direction — same shape as the Phase B app-launch path, so it's behaviorally integrated, not purely observational. Slices 3/4 (`close_focused`, `navigate`) submit commands and log effects but do NOT yet consume them to drive focus, because `HostModel` allocates its own pane IDs independent of the egui tile IDs `PlexiApp` tracks — so `PaneClosed`/`FocusChanged` effects currently reference HostModel's private pane list, not the real tiles. This is honest tech debt: the observation layer exists, but ID reconciliation is required before effects can drive real focus transitions. Every user-facing pane op (launch, split, close, navigate) now flows through `HostCommand`.

**Breaks if:** the debug log for `.plexi-v3/plexi.log` stops showing "split_focused effects", "close_focused effects", or "navigate effects" lines when the user triggers the corresponding keybindings. The `Direction` → `crate::host::command::Direction` mapping in `navigate()` drops or reorders a variant.

## 2026-04-18 — [CHANGED] Cutover slice 1: launch_app_by_id routes through HostModel (PRs 3e162f2, 27e0ece, 621fe79 → v3)
First vertical slice of the `PlexiApp` → `HostModel` cutover. `PlexiApp` now holds `host: HostModel` + `host_services: HostServices`. App launches submit a `HostCommand::OpenPane` to `HostModel`, observe the returned `PaneOpened` effect, and use its `share` + `placement` fields to drive `egui_tiles` insertion. The legacy 3:1 hardcode in `pane_ops::split_with_new_pane` is gone — the function now takes a `ShareRatio` parameter. App-launch default is 1:1 (50/50). Terminal `split_focused` retains its own inline layout (migrates next session).

Added `initial_share: Option<f32>` to manifest capabilities and `AppRegistry::share_for(app_id)` accessor. Launch path reads the manifest share, validates (0.0, 1.0) exclusive, and converts to `ShareRatio`. Invalid shares log a warning and fall back to 0.5. Example manifests updated: quick-note 0.3, snake 0.5, wikipedia 0.6, todo 0.4, audio-recorder 0.3, video-player 0.7. File-browser is a Rust-native builtin with no manifest and uses the 0.5 default.

**Breaks if:** file browser opens at 75/25 again (pane_ops 3:1 regression). Example apps at `examples/*/manifest.toml` ignore their `initial_share` when launched. `cargo test --release` `host::harness::tests::open_pane_carries_share_to_effect` fails. `share_ratio_from_fraction` accepts a fraction <= 0 or >= 1 without logging a warning.

## 2026-04-18 — [CHANGED] Plexi SDK: widgets foundation + headless snapshot testing
Converted `sdk/python/plexi_sdk.py` into a package (`plexi_sdk/__init__.py` holds the original content byte-identical). Added `plexi_sdk.widgets.{ScrollState, TextBuffer, TextArea, TextAreaTheme}` — pure-Python reusable text editor primitives with zero deps. Added `plexi_sdk.testing` + new Rust bin `plexi_render` for headless PNG snapshot tests (stdlib-only PNG decode, pixel assertions). Added `pyrightconfig.json` at repo root so IDEs resolve `plexi_sdk` imports. 81 Python tests pass (25 scroll + 44 text_buffer + 5 snapshot + 7 text_area); all 49 Rust unit/integration tests still green (pre-existing doctest failure in headless_renderer.rs docstring untouched). This work sits above the PGAP protocol and survives the `PlexiApp` → `HostModel` cutover untouched. Text-editor and file-explorer as apps both build on these primitives.
**Breaks if:** `from plexi_sdk import App, RenderContext, BG` fails in any example app. `cargo build --bin plexi_render --release` fails. `python3 -m pytest sdk/python/tests/` has any failing test.

## 2026-04-18 — [CHANGED] Remove audio/video subsystems; keep typed pipes

Deleted `src/media/audio.rs` and `src/media/video.rs`. Replaced `src/media/mod.rs` with a stub comment pointing to `typed_pipes.rs`. Removed `AudioPlay`, `AudioCapture`, `VideoPlayer`, `AudioMeter` from `DrawCommand` and all routing/render code in `ProcessApp`. Removed `AudioRecord`, `AudioPlayback`, `VideoPlayback` capabilities from `Capability` enum. Stripped `audio_capture`, `audio_play`, `video_player`, `audio_meter` methods from all `plexi_sdk.py` copies. Typed pipes infrastructure (`src/typed_pipes.rs`) untouched.

**Breaks if:** `cargo test` drops below 57 passing (was 57 after this change), or `PipeOpen`/`PipeSend` stop routing correctly in `ProcessApp`.

## 2026-04-18 — [CHANGED] inject_state + net.http brokering (PGAP v3)

Added `PlexiEvent::InjectState { payload: Value }` to the protocol. SDK calls `on_inject(ctx, payload)` synchronously on the PGAP loop thread — no key-pushing needed to drive app state in tests.

Added `http_request` / `http_response` PGAP channel. Apps call `emit.http_get(url)` from any thread (emits `http_request`, blocks on a queue). SDK handles `http_response` by unblocking the caller. Wikipedia app migrated from inline `urllib.request` to this channel.

`Harness` gains `inject_state(payload)` and `mock_http(url, body)`. `render_frame` pre-drains buffered `http_request` commands before sending the render event — this eliminates the timing race where `on_render` would see stale state if the render arrived before the http_response.

**Breaks if:** `wikipedia_inject_state_shows_results` fails to find "Rust" in rendered text, or `wikipedia_http_mock_intercept` panics waiting for `frame_done`.

## 2026-04-18 — [CHANGED] Wire harness — agent dev loop produces PNG end-to-end (Layer 3)

Added `render_pgap_frame(&[Value], width, height) -> Vec<u8>` to `HeadlessRenderer`. Parses PGAP wire format (CSS hex colors, flat JSON) directly — `rect`, `text`, `line`. `frame_done` and unsupported commands silently skipped.

Added `Harness::render_to_png` in `pgap_test_harness.rs` — wraps `render_frame` + `render_pgap_frame` in one call. This is the agent dev loop API: spawn app → `render_to_png` → inspect/assert → iterate.

`agent_dev_loop_produces_png` test: spawns snake subprocess, renders a frame, asserts output is valid PNG with visible pixels. 75/75 tests pass.

**Breaks if:** `agent_dev_loop_produces_png` fails, or `Harness::render_to_png` is removed, or the headless renderer drops PGAP command support.

## 2026-04-18 — [CHANGED] HostModel rebuild — full command/effect set (Layer 2)

Rewrote all five `src/host/` files test-first. 26 tests covering every command and effect.

New commands vs the stub: `Navigate(Direction)`, `SplitHorizontal`, `SplitVertical`, `NewContext`, `SwitchContext`, `SendKeyToFocusedApp`, `SimulatePathChanged`, `CheckCapability`, `GrantCapability`, `DenyCapability`. New effects: `SplitOpened`, `ContextCreated`, `ContextSwitched`, `AppKeyDispatched`, `PathBroadcasted`, `CapabilityGranted`, `CapabilityDenied`, `CapabilityPromptRequired`, `EventEmitted`.

`HostServices` now has a `Box<dyn EventSink>` trait object (`NoopEventSink` default, `VecEventSink` for tests). `HostPane` tracks `declared_capabilities` and `group`. `HostContext` tracks `groups` and `permissions`.

**Breaks if:** `cargo test host` drops below 26 passing tests, or any host file imports egui.

## 2026-04-18 — [CHANGED] Headless PNG renderer shipped (Layer 3)

`src/headless_renderer.rs` — `View::Canvas` → PNG via `tiny-skia` + `fontdue`. `HeadlessRenderer::render_to_pixmap` and `render_to_png`. Three tests: rect pixel assertion at exact coordinates, text rendering without panic, Document view → blank frame. No egui dependency. 50/50 tests pass.

Added `tiny-skia = "0.11"` and `fontdue = "0.9"` to Cargo.toml. Bundled `fonts/DejaVuSans.ttf` used for text rasterization via `include_bytes!`.

**Breaks if:** `cargo test headless` fails, or `src/headless_renderer.rs` gains an egui import.

## 2026-04-18 — [DECISION] Doc overhaul + E2E testing architecture

Rewrote doc layer to match the real north star. Key decisions:

- `STATE_OF_PLEXI.md` → `ARCHITECTURE.md`. Removed temporal sections (port reality check, critical path checklist) — those belong in git log and DEV_LOG. Architecture doc should be timeless.
- Deleted `VISION.md`, `V3_PROGRESS.md`, `docs/PRD-mvp.md`, `docs/PRD-future.md`, `docs/architecture-audit.md`, `docs/mvp-interaction-spec.md`, `docs/future-enhancements/` (6 files). All were pre-PGAP era or tracking docs with no permanent value.
- Created `docs/specs/subsystems/host-architecture.md` and `testing-infrastructure.md` — these are the missing specs for the HostModel pure state machine, renderer layer, three-layer test strategy, and agent dev loop.
- Rewrote `docs/AGENTS.md` completely — old version described Tauri + Playwright + TypeScript (pre-egui era).

**E2E testing architecture decided:**
1. **Headless PNG renderer** (`src/headless_renderer.rs`, tiny-skia) — draw commands → PNG, no egui. Unblocks agent dev loop.
2. **HostModel rebuild** (`src/host/` gutted and rebuilt) — pure state machine, test-first via HostHarness, mocked HostServices at every real-system boundary. Existing `src/host/` is a stub with 5 commands; needs full command/effect set.
3. **Wire harness** — extend `pgap_test_harness` to call headless renderer, producing PNGs after `render_frame()`.

**WASM decision:** Python subprocess (honor-system capabilities) for v3.0. WASM v3.1+ for Rust apps when toolchain is ready. Protocol interface already maps cleanly to WASM component exports (init/render/on_key as typed functions) — transport change only, no protocol redesign.

**Anti-stub rule added to CLAUDE.md:** define done by the test, not the code. No partial merges. HostHarness tests written before HostModel implementation.

## 2026-04-18 — [CHANGED] Codebase refactor: module splits, unified error type, PGAP reference doc

Split the two largest files into focused module directories. `process_app.rs` (1319 LOC) → `process_app/mod.rs` (590) + `routing.rs` (420) + `render.rs` (149) + `prompts.rs` (102). `app.rs` (1062 LOC) → `app/mod.rs` (864) + `dispatch.rs` (118) + `sync.rs` (85). Each sub-file has a single responsibility: routing dispatches DrawCommands to subsystems; render translates committed frames into egui calls; prompts owns the capability/secret modal UI; dispatch owns keyboard routing + AppCommand execution; sync owns CWD polling + PathChanged broadcast.

Added `src/error.rs` with a `PlexiError` thiserror enum covering Io, Protocol, Permission, AppLaunch, Media, Pipe, Registry, NotImplemented — establishes a single error vocabulary for future refactoring of fragmented return types (currently `std::io::Error`, `String`, `Option<>` scattered across modules).

Added `docs/pgap-reference.md` — canonical PGAP protocol reference covering all PlexiEvent and DrawCommand variants, handshake sequence, typed-pipe binary format, capability flow, manifest.toml schema, and SDK quick-start. No developer should need to read Rust source to build a Plexi app.

All 39 tests pass; all 7 smoke-test app handshakes green post-install.

**Breaks if:** `cargo test` fails any of the 39 tests. OR: `just install-v3` smoke test fails any handshake.

## 2026-04-18 — [CHANGED] Keyboard ownership, pane management, spawn.app, events.jsonl init

**Keyboard ownership**: Apps now declare `keyboard_capture = true` in `manifest.toml`. When set, all host shortcuts except Cmd+Q (quit) and Cmd+W (close pane) are suppressed while that app is focused. The `keyboard_capture()` method is on the `App` trait (default false); `ProcessApp` reads from manifest and returns it. `poll_actions` now takes `keyboard_capture_active: bool` as second param and gates via early return inside `input_mut()`.

**Pane management**: Added `layout_hint: Option<String>` to `AppCapabilities` manifest schema. Values: `"split"` (default, linked terminal) or `"overlay"` (full pane, no terminal). `launch_app_by_id` now routes through `launch_app_by_id_with_layout` which reads this hint. `close_focused_app` was bypassing `close_tile` for the linked terminal pane — fixed to route through `close_tile` so sibling focus transfer runs correctly.

**spawn.app DrawCommand**: `DrawCommand::SpawnApp { type_id, layout }` added to protocol. `PlexiEvent::AppSpawned { pane_id, type_id }` added as confirmation. `ProcessApp` pushes `AppCommand::SpawnApp` to pending_commands; `dispatch_app_key_events` returns these deferred (since they need host-level access); `update()` handles them and sends `AppSpawned` back via `queue_outbound_event()` on the `App` trait.

**events.jsonl**: `event_log::init_global` now called in `PlexiApp::new`. Events are written to `~/.plexi-v3/events.jsonl` (and `.plexi/events.jsonl` if inside a workspace).

**Fibonacci POC**: Example app at `~/.plexi-v3/apps/fibonacci/`. Declares `keyboard_capture = true` and `spawn.app` capability. On first render, auto-spawns the next Fibonacci pane via `SpawnApp`. Chain stops at index 10. Passes PGAP handshake smoke test.

**Breaks if:** Focused app with `keyboard_capture = true` in manifest still lets Cmd+HJKL fire (keyboard ownership not working). events.jsonl file not created in `~/.plexi-v3/` after first launch. `close_focused_app` leaves zombie linked terminal pane after close (focus doesn't transfer to sibling).

## 2026-04-18 — [CHANGED] HostEvent enum aligned to spec §6.1

The event bus was implemented with variant names that drifted from the spec (NotificationEmitted vs NotificationPosted, RunCreated vs RunStarted, PermissionPrompted vs PermissionDecision) and two non-spec variants (ApiCall, CostReport) were forward-declared but never emitted. The spec §6.1 is SSoT per CLAUDE.md, so the code is the thing that moves.

Renames: NotificationEmitted → NotificationPosted, NotificationActioned → NotificationActionInvoked, RunCreated → RunStarted, PermissionPrompted → PermissionDecision (also moved from before-prompt to after-decision so it carries `granted: bool`). Added SecretPrompted / SecretDenied / PipeOpened / PipeClosed. Dropped ApiCall (no host-side HTTP broker exists) and CostReport (redundant with AgentTurn's new cost_cents field). AgentTurn now carries `pane_id, tokens_in, tokens_out, cost_cents` per spec, with cost derived from LedgerRow rounding to whole cents. PipeWrite removed — the write path was firing one event per audio frame, which would have flooded the log during captures; PipeOpened at allocation + PipeClosed at teardown is sufficient.

Guard test in `event_log::tests::host_event_wire_shape_matches_spec` locks the full variant set and JSON kind tags. Any future rename has to land in the spec first, then this test.

**Breaks if:** `grep -E 'NotificationEmitted|NotificationActioned|RunCreated|PermissionPrompted|PipeWrite|ApiCall|CostReport' src/` returns a hit outside a comment or migration note. OR: `cargo test host_event_wire_shape_matches_spec` fails.

## 2026-04-18 — [FIX] Drain thread blocking accept() deadlocked close() on failed start_capture

After the `todo!()` fixes landed, pressing R in the audio recorder still froze the host. Root cause: when `start_capture` returns `Err`, the host calls `pipe_registry.close("pipe-id")`, which sets `shutdown=true` and joins the drain thread. But the drain thread was sitting in a blocking `listener.accept()` waiting for the app to connect — and the app never does, because `PipeOpened` was never emitted. `accept()` doesn't observe `shutdown` → `join()` blocks forever → UI thread freezes.

**Fix:** `listener.set_nonblocking(true)` in `open_binary`, then rewrite the drain thread's accept as a 50ms poll loop that checks `shutdown` on `WouldBlock` and exits cleanly. On successful connect, switch the returned stream back to blocking mode for the write loop.

**Regression test:** `typed_pipes::tests::close_without_client_does_not_deadlock` opens a binary pipe, closes immediately, asserts `close()` completes in <2s.

**Breaks if:** a failed `AudioCapture` / `VideoCapture` DrawCommand (or any path that calls `pipe_registry.close()` on a pipe the app never connected to) freezes the UI for more than a second. OR: `cargo test typed_pipes::tests::close_without_client_does_not_deadlock` takes >2s or times out.

## 2026-04-18 — [GOTCHA] `todo!()` in a prod factory froze the host GUI

`CoreAudioDevice::start_capture` was `todo!("Layer 4")`. Compiled clean, passed every test (harness tests set `PLEXI_AUDIO=mock://`), then panicked the UI thread the first time the user pressed R in the audio recorder — freeze → force quit. Same pattern in `AvfVideoDecoder` (four `todo!()` methods).

**Root cause:** factory functions can return an impl whose trait methods panic. Tests that go through mock variants never touch the panicking path.

**Permanent fixes:**

1. `#![deny(clippy::todo, clippy::unimplemented)]` in `src/main.rs` — clippy gate blocks new `todo!()` in non-test code.
2. `prod_stub_tests` modules in `src/media/audio.rs` and `src/media/video.rs` — call every trait method on the prod impl, assert no panic. Catches the bug even without clippy.
3. `scripts/smoke-test.sh` (wired into `just install-v3`): feeds PGAP Init to each installed app and asserts `ready` within 3s, then launches host for 2s and scans the log for panics. First post-install gate that exercises the real built bundle.

**Do NOT:** leave `todo!()` or `unimplemented!()` in any factory-returned impl. A stub returns `Err(NotImplemented)`, `None`, or a noop — never a panic.

**Breaks if:** `scripts/smoke-test.sh` after `just install-v3` reports anything other than green for all 6 apps and the host-launch check. OR: `cargo clippy --all-targets -- -D clippy::todo -D clippy::unimplemented` finds a violation in non-test code.

## 2026-04-18 — [CHANGED] Pane groups + PathChanged broadcast + PGAP test harness (v3 critical-path #10 + #13)

Added an opt-in `group` field to app manifests (`[app.capabilities] group = "cwd"`). At launch, an app inherits its group on the host `TerminalPane`. `App::sync_app_cwd` now polls each linked terminal's CWD via lsof, diffs against `last_synced_cwd`, and on change (a) sends `PlexiEvent::PathChanged { cwd }` to the source pane's app and (b) broadcasts the same event to every OTHER pane sharing the group. Todo app opts into `"cwd"` and reloads `./.plexi/todos.json` on PathChanged so its list tracks the focused terminal.

PGAP protocol test harness lives at `src/pgap_test_harness.rs` (inline `#[cfg(test)]` module — egui binary crate has no lib target). Spawns each example app as a subprocess, drives NDJSON over stdin/stdout, asserts handshake + render. Nine tests: six app handshakes, snake frame render, todo PathChanged reload, MockAudioDevice WAV round-trip, MockVideoDecoder RGBA frames. CI gate at `.github/workflows/plexi-v3-test.yml` runs on push/PR to `v3` with `PLEXI_AUDIO=mock://` + `PLEXI_VIDEO=mock://`.

Secrets scope (`workspace_root`) is unchanged by PathChanged — only the app's tracked cwd moves. `last_synced_cwd` on TerminalPane guards against per-frame PathChanged spam.

**Breaks if:** Launching the Todo app in v3, then `cd`-ing in a linked terminal, does NOT update the path displayed in the Todo header and does NOT reload `.plexi/todos.json` from the new directory. OR: `cargo test pgap_test_harness` fails any of the 9 tests on a host with `python3` on PATH.

## 2026-04-16 — [CHANGED] Layer 5: Python SDK v3 + six example apps

Rewrote `sdk/python/plexi_sdk.py` from the v2 decorator pattern to a subclass pattern (`class MyApp(App)`). Key changes: `App.run()` now handles the PGAP v3 `Init` handshake (protocol validation, `Ready` reply); `RenderContext` carries `frame_id`, `rect`, `workspace_root`, `capabilities`, `feature_flags`; `FrameDone` is auto-emitted with `frame_id`; `Emitter` gains blocking `capability_request()`/`secret_get()` via `queue.Queue` + background stdin thread dispatch; `Pipe` class covers both binary (unix socket, length-prefixed frames) and JSON-mode pipes.

Six apps shipped: `snake`, `wikipedia`, `todo`, `audio-recorder`, `video-player`, `quick-note`. All tested via stdin Init injection → stdout Ready. SDK copied into each app dir (no symlinks — avoids PYTHONPATH fragility). Cargo check and 18/18 tests unaffected (no Rust touched).

The v2 alpha SDK used decorators (`@app.on_render`). The v3 subclass pattern was chosen for clarity in the spec examples and to make state management natural (instance variables on `self`). The decorator pattern is still possible but adds indirection without benefit in an SDK where apps are typically one class.

**Breaks if:** Any of the six apps fails to print `{"type":"ready",...}` as the first stdout line when fed a PGAP v3 Init event on stdin.

## 2026-04-11 — [CHANGED] Secrets manager write UI, index-file listing, logging infrastructure

Secrets manager upgraded from read-only viewer to full add/delete UI. Listing fixed by replacing `security dump-keychain` (triggers invisible macOS permission prompt) with a local `secrets-index.json`. Centralized file logging added via `fern` with config-driven log levels and `DrawCommand::Log` forwarding from external apps.

**Progress:** Secrets manager: `n` adds (masked value, dir pre-filled from CWD), `d` deletes, optimistic in-memory updates, `app_id` aligned to `"plexi-run"` for CLI consistency. Index file at `~/.plexi-alpha/secrets-index.json` maintained by `store_secret`/`delete_secret`. Logger writes to `~/.plexi-alpha/plexi.log`, 10MB rotation, level from `[log]` in `config.toml`. External app stderr piped + forwarded as warn. Python SDK gains `emit.info/warn/error/debug`. App workspace restore now uses manifest permissions not `AppPermissions::builtin()`.
**Open:** Directory-scoped workspace persistence (`.plexi/workspace.json`) still deferred. File browser async worktree fix not merged to alpha. SpacetimeDB shared workspace PoC in memory but not started.

## 2026-04-11 — [CHANGED] Where Were We snapshot
File browser async I/O fix (background thread for `refresh()`), Wikipedia and Plexi Browser apps built and installed, `plexi.json` manifest spec written, app install paths clarified by build variant.
**Progress:** File browser no longer blocks UI thread on directory navigation (uses `mpsc` channel + background thread). Wikipedia and Plexi Browser apps installed to `~/.plexi-alpha/apps/`. `docs/plexi-json-spec.md` + JSON Schema written. `CLAUDE.md` updated with app install path table per build.
**Open:** Wikipedia and Plexi Browser apps not yet smoke-tested in the running alpha build. Server test (`plexi-browser/server.py` + curl) not verified by user. File browser async fix built in a worktree — not merged to alpha branch yet.

## 2026-04-10 — [FIX] ProcessApp now forwards Event::Text for typed characters; letter key protocol is lowercase

`process_app.rs` `handle_key` only forwarded `egui::Event::Key`, which uses PascalCase enum variant names (`"J"`, `"K"`). Typed characters arrive via `egui::Event::Text` — those were never forwarded, so text input (search queries, URL bars) never worked in subprocess apps.

Fix: add `Event::Text` forwarding, sending each printable char as `PlexiEvent::Key`. To avoid double-firing letters (egui fires both `Event::Key { key: Key::J }` AND `Event::Text("j")` for the same press), bare letter keys (A–Z, no modifiers) are suppressed from `Event::Key` forwarding. Modifier-held combos (Cmd+S, Ctrl+C) still come via `Event::Key` since egui never fires `Event::Text` for those.

**Protocol contract:** Printable chars arrive lowercase/proper-case as single-char strings. Control keys (`"Backspace"`, `"Enter"`, `"ArrowDown"`, etc.) arrive as PascalCase. Modifier combos arrive uppercase PascalCase. Updated all apps: `"j"/"k"/"r"` instead of `"J"/"K"/"R"` in git-log, process-monitor, wikipedia.

## 2026-04-10 — [ADDED] plexi.json manifest spec — declarative app format and /.well-known/ discovery

Added `docs/plexi-json-spec.md`, `schemas/plexi-manifest-schema.json`, and `examples/wikipedia/plexi.json`. The format serves two modes: local declarative apps (no code needed) and website discovery via `/.well-known/plexi.json` (RFC 8615). Key design decisions: static mode (no `endpoint`) renders the `draw` array once with scroll only — no subprocess, no network. The `draw` array reuses the existing draw protocol vocabulary exactly (`rect`, `text`, `line`, `list`, `frame_done`) so static and dynamic apps are consistent. `display` enum (`standalone` | `panel` | `overlay`) borrowed from PWA manifest. Permissions follow `domain[.access]` pattern matching the existing capability system (`filesystem.read`, `filesystem.write`, `network`, `terminal`, `secrets`). Discovery uses `X-Plexi-Client: 1` request header so servers can distinguish Plexi from browsers. Schema is JSON Schema draft-07 with strict `additionalProperties: false` on draw command objects to fail fast on typos.

## 2026-04-10 — [ADDED] Secrets Manager builtin app (read-only vault viewer)

Added `secrets_app.rs` — a read-only viewer for all Plexi Keychain secrets. Opens fullscreen (no terminal split) via `Cmd+Shift+S`, toggles closed on repeat. Parses `security dump-keychain` output via new `list_all_secrets() -> Vec<SecretEntry>` in `secrets.rs`, splitting the account string `"{app_id}/{directory}/{key}"` at first and last slash. j/k navigation, r to refresh, no inline add/delete to keep attack surface minimal. Wired into workspace restore under the `"secrets_manager"` type_id arm.

## 2026-04-10 — [FUTURE] Collaborative state via SpacetimeDB + append-only snapshots

The `serialize_state()`/`restore_state()` contract on the App trait is transport-agnostic — JSON in, JSON out. This means collaborative features could be layered in by replacing disk read/write with SpacetimeDB table subscriptions. Each pane's state = a row. Mutations push deltas to subscribers. Apps don't know they're collaborative. Additionally, snapshotting state every ~5 seconds as append-only rows gives full rewind/undo history across restarts for free. Locally, the same pattern works as an append-only JSON log file. v1 conflict resolution: last-write-wins on full state blob. CRDTs or OT per app type would come later. Not building now — the foundation supports it without changes.

## 2026-04-10 — [DECISION] Directory-scoped workspace persistence is the next step

Current workspace saves to `~/.plexi/workspaces/default.json` (global). The next concrete step is saving to `.plexi/workspace.json` in the current project directory instead. This unlocks: shareable project folders (share the dir, other person opens Plexi, layout restores), git-trackable workspace state, and the spatial zoom vision where navigating into a `.plexi/` directory restores that project's context. The `serialize_state()`/`restore_state()` App trait methods already handle per-app state — just need to change where the file is written. Gotchas to watch: multiple Plexi instances in same directory (file locking), binary files in git (audio/video — use LFS or .gitignore), and relative paths in serialized state (apps should not store absolute paths).

## 2026-04-10 — [CHANGED] App+terminal refactored from embedded bar to separate panes

The embedded terminal command bar (fixed 72px at bottom of app pane, animated opacity) was abandoned after testing. Scroll events didn't propagate through `allocate_new_ui`, click-to-focus was awkward, and the embedded terminal was too small to be useful. Replaced with auto-split: opening an app creates a real vertical split (75% app / 25% terminal) using the existing tile tree. Both are normal panes with natural resize, focus, and zoom behavior. Tab navigates down from app to terminal. Cmd+K navigates back up. Escape closes the app and collapses the split. This means `SurfaceMode::AppActive` now renders the app full-height with no embedded terminal at all — the old `COMMAND_BAR_HEIGHT`, opacity animation, and divider code in tiling.rs is dead.

## 2026-04-10 — [DECISION] Two-way CWD sync via lsof polling, not OSC 7

Tried emitting OSC 7 escape sequences to the PTY to track directory changes. The shell printed the raw escape as text because OSC 7 is an output-direction protocol (shell→emulator), not input (emulator→shell). Removed OSC writes entirely. Instead: file browser→terminal uses `AppCommand::Cd` which writes `cd path\n`. Terminal→file browser uses `shell::get_pid_cwd(child_pid)` (lsof on macOS) polled each frame in `sync_app_cwd`, same mechanism as beta/v2. The `sync_cwd` method on the App trait allows any app to respond to terminal directory changes.

## 2026-04-10 — [GOTCHA] allocate_new_ui breaks ScrollArea mouse events

The sidebar layout initially used `ui.allocate_new_ui()` with manual rect geometry for the two-column file browser (list + preview). Mouse wheel scrolling didn't work — events weren't propagated to the ScrollArea inside the allocated UI. Switched to `ui.columns(2, ...)` which is what beta/v2 uses and works correctly. Lesson: prefer egui's built-in layout primitives over manual rect allocation when scroll interaction is needed.

## 2026-04-10 — [ADDED] Capability-gated permission system and secrets management

Built in one session with 4 parallel agents: `secrets.rs` (macOS Keychain via `security` CLI, directory walk-up resolution), `app_api.rs` (structured ListDir/ReadFile/WriteFile/SecretGet/SecretStore with path-scope enforcement), `cli.rs` (`plexi run` reads `.plexi/commands.toml`, injects secrets as env vars), `app_registry.rs` extended with capability declarations. `app_permissions.rs` gates every `AppCommand` through `check_command()` — sandboxed apps can't escape their launch directory or write to the terminal without explicit permission. Built-in apps are pre-approved. The protocol spec is at `docs/specs/app-infrastructure.md`.

## 2026-04-10 — [GOTCHA] handle_key must check modifiers to avoid swallowing Plexi shortcuts

The file browser's `handle_key` consumed Enter, H, L, Backspace unconditionally. This swallowed Cmd+Enter (zoom toggle) and Cmd+H/J/K/L (pane navigation). Fix: guard all non-modifier keys with `!input.modifiers.command`. This is a general rule for all apps: Cmd-modified keys belong to Plexi, not the app.

## 2026-04-09 — [DECISION] App focus uses SurfaceLayer enum + animated dim, not a split-pane model

The original plan for app+terminal coexistence had three `SurfaceMode` variants: `FullTerminal`, `AppWithCommandBar`, and `AppWithTerminalSplit` — Tab would toggle between the last two. Dropped in favour of two modes (`FullTerminal` / `AppActive`) with a separate `SurfaceLayer` enum (`App` / `Terminal`) tracking which surface owns keyboard focus. Tab toggles `focused_surface` rather than changing pane geometry. When the terminal has focus, the app dims to `APP_DIM_OPACITY = 0.45` via `animate_value_with_time` (0.15s). The divider line switches from `bg_active` to `accent` as an additional focus cue. Reason: the split-pane approach added geometry complexity and a third rendering path; the dim-and-focus approach gives the same UX signal with zero geometry change and is simpler to reason about.

## 2026-04-09 — [ADDED] File browser rewritten with vector icons and sidebar preview

`file_browser_app.rs` rewrote from a plain 20px monospace list to match the beta/v2 `CanvasPane` style: 58px rows with vector-drawn file type icons (folder tab+body, image mountain, audio speaker, markdown pen, code brackets, config sliders, PDF label, archive grid, generic lines), `format_size`/`format_modified` subtitles, and a 920px+ sidebar preview panel (image texture preview, directory stats, text preview, generic metadata). Keyboard nav extended to J/K/H/L, Backspace (parent), Home/End, and Enter (open). `image` crate added to Cargo.toml for texture loading. Sidebar uses `allocate_new_ui` with manual rect geometry (55/45 split) rather than `ui.columns()` because columns don't allow independent scroll areas.

## 2026-04-09 — [GOTCHA] pane_ops method name diverged from TerminalPane after action rename

`keys.rs` renamed `ToggleTerminalSplit` → `ToggleAppFocus` and `pane.rs` renamed `toggle_terminal_split()` → `toggle_surface_focus()`, but `pane_ops.rs` kept the old method `toggle_focused_terminal_split()` calling the old method name. Build would have failed if the rename on `TerminalPane` was complete. Always grep for the old name across all files when renaming a method — the pane_ops wrapper layer is easy to miss since it's a thin delegation and doesn't appear in the action handler directly.

## 2026-03-25 — [GOTCHA] File drop target must use geometric hit test, not focus state

The initial fix for duplicate file drops (guarding `dropped_files` with `has_focus` in view.rs) caused drops to land in the wrong pane. Root cause: `focused_tile` in `PlexiBehavior` is derived from `ctx.focused_pane`, which is updated AFTER `tree.ui()` completes — so it's always 1 frame behind the actual hover detection (`new_focused`). On the drop frame, `has_focus` could point to the previously-focused pane, not the one under the cursor.

Fix: moved drop handling from `view.rs` into `pane_ui` in `tiling.rs`, using the same `drag_cursor_pos` / `max_rect().contains(pos)` hit test as hover detection. Also extended `has_drag` to check `dropped_files` (not just `hovered_files`) so `drag_cursor_pos` is computed on the drop frame. Lesson: when an action must target the pane under the cursor, use the geometric hit test directly — never rely on focus state, which has inherent frame delay.

## 2026-03-25 — [FIX] File drag focus was slow (~500ms) because no repaints were requested

During an external file drag, winit on macOS only fires `HoveredFile` once (on `draggingEntered:`). No `CursorMoved` events fire during the drag. The app already worked around this by querying `NSWindow.mouseLocationOutsideOfEventStream()` each frame — but "each frame" only meant every ~530ms (cursor blink timer) when the terminal was idle. This made focus tracking during drags feel sluggish (0.5–1.5s delay) and caused focus to "stick" on panes with active PTY output (like Claude Code) since those triggered more frequent repaints.

Fix: `ui.ctx().request_repaint()` when `hovered_files` is non-empty. This is the idiomatic egui approach — there is no continuous repaint mode or drag-specific hook. The repaint loop is self-terminating: it only runs while files are being dragged. `hovered_files` persists across frames (egui clones it in `RawInput::take()`, unlike `dropped_files` which uses `mem::take`), so the check stays true for the duration of the drag.

## 2026-03-25 — [DECISION] V1 gate cleared — moving to P2 polish

All P1 blockers resolved: code smell refactor complete, unit/integration tests added, and #56 (copy not preserving newlines) fixed. Remaining open issues are P2–P4. Rather than shipping V1 immediately, picking up #54 (drag screenshot duplication across panes) and #53 (Open Config menu item) as quality-of-life polish before the release. These aren't blockers but they're the kind of rough edges early adopters will hit.

## 2026-03-24 — [FIX] Apple Symbols loaded at runtime for missing glyph coverage

The existing font chain (JBM Nerd Font → DejaVu Sans → egui defaults) still left some characters as squares in the terminal — specifically symbols from ranges like Miscellaneous Technical (⌥ ⌘ ⏺ etc.), Geometric Shapes, and Dingbats used by Claude Code, Starship, and similar CLI tools. JBM Nerd Font covers the Nerd Font PUA but not all of these standard Unicode ranges, and DejaVu is focused on Latin extended / Braille.

Fix: load `/System/Library/Fonts/Apple Symbols.ttf` at runtime and insert it at position 2 in both Proportional and Monospace family chains (after JBM and DejaVu, before egui's bundled Ubuntu/NotoEmoji). The font is loaded with `std::fs::read` so it adds zero binary size and silently skips if not found. Apple Symbols is always present on macOS 10.x+, making this safe. Bundling was rejected — 23M Arial Unicode and 900k Apple Symbols adds bloat; runtime loading is cleaner.

## 2026-03-24 — [FIX] File drag target now follows cursor across panes

winit-0.30.13's macOS impl only fires `WindowEvent::HoveredFile` in `draggingEntered:` — there is no `draggingUpdated:` handler. This means egui never receives `CursorMoved` events during an external file drag, leaving `pointer.hover_pos()` stale at the drag-entry position. The previous fix (#23) worked for the first pane but not when moving between panes. Fixed by querying `NSWindow.mouseLocationOutsideOfEventStream()` each frame when `hovered_files` is non-empty, converting from AppKit window-base coords (Y-up, bottom-left origin) to egui coords (`egui_y = content_height - base.y`). Requires the `NSWindow` feature on `objc2-app-kit`. The native position is used for the pane hit-test (`max_rect().contains(pos)`) instead of the stale `rect_contains_pointer`.

## 2026-03-23 — Install now builds a proper .app bundle via cargo-bundle

`just install` switched from manually assembling the `.app` directory to using `cargo bundle --release`, which generates the bundle from `Cargo.toml` metadata (matching what `install.sh` does for fresh installs). Binary is also copied to `/usr/local/bin/plexi` for CLI access. Don't revert to manual mkdir/cp — `cargo bundle` keeps `Info.plist` in sync with `Cargo.toml` automatically.

## 2026-03-23 — [FIX] Gemini/Ink rendering issues came from missing terminal protocol support plus font-rasterized block glyphs

The black Gemini input bars were not a generic background-paint bug: Gemini queries `OSC 11` for the terminal background color and fell back to `black` because `egui_term` dropped `alacritty_terminal::Event::ColorRequest` instead of replying on the PTY. Fixed by wiring dynamic color responses through the backend and exposing Rebecca's foreground/background as terminal dynamic colors. The faint seams around Gemini's half-block borders and the Claude-style block logo were a second issue: `▀`, `▄`, `█`, and quadrant blocks were being rendered through the font path, which introduced antialiasing seams that Ghostty avoids by drawing geometry directly. Added a primitive block-element renderer in `deps/egui_term/src/graphics.rs` and changed render-time cell geometry to derive from the actual layout rect instead of truncated integer cell sizes, which removed the remaining right/bottom gutter artifacts. The important lesson is to treat terminal protocol round-trips and Unicode graphics elements as core emulator behavior, not per-app compatibility hacks.

## 2026-03-23 — [FIX] Washed-out TUI colors were mostly a theme mismatch, not a renderer bug

Plexi was being compared against Ghostty while hardcoding a Catppuccin Mocha terminal palette, but the local Ghostty install was actually running `theme = Rebecca`. Swapped Plexi's terminal theme to Rebecca (matching Ghostty's background and ANSI 0-15 colors) and the washed-out look in `btop` corrected immediately. Important lesson: before debugging color math, verify both terminals are using the same palette; otherwise "renderer mismatch" can be a false lead.

## 2026-03-23 — [GOTCHA] Ghostty TERM/terminfo parity did not fix Gemini CLI's black input bars

Tried matching Ghostty's terminal identity by exporting `TERM=xterm-ghostty` plus `TERMINFO` pointing at Ghostty's bundled terminfo when available, with fallback to `xterm-256color`. This changed Plexi's advertised capabilities to line up with Ghostty, but it did not change Gemini CLI's black wrapping/input-bar behavior. That strongly suggests the remaining Gemini issue is not terminfo-driven; it is more likely tied to how `egui_term` paints cell backgrounds for ANSI black/default background combinations.

## 2026-03-23 — [CHANGED] Pane wrapper background was still using old Catppuccin color

After switching the terminal palette to Rebecca, the pane/frame wrappers and zoomed-pane wrapper were still hardcoded to the old Catppuccin terminal background (`#1e1e2e`). This caused a thin inner band/padding region around terminals to render in the wrong shade even when the terminal content itself was correct. Added a shared `Colors::TERMINAL_BG` constant for Rebecca and updated the pane wrappers to use it. This fixes app chrome mismatch around the terminal, but does not resolve Gemini CLI's black per-row bars.

## 2026-03-23 — [FUTURE] Claude CLI missing square/icon is probably a separate font fallback issue

While investigating Gemini colors, Claude CLI still showed a square/placeholder where an icon or glyph should likely render. This does not appear related to the color pipeline work above and should be treated as a separate font/glyph fallback investigation after the Gemini background issue is solved.

## 2026-03-23 — Investigating washed-out / gray terminal colors vs Ghostty

**Problem:** TUI apps (btop, Gemini CLI) look washed out and gray in Plexi compared to Ghostty. The Gemini CLI input box also shows black padding that doesn't match the terminal background.

**Attempted fix (reverted):** Added bold→bright color promotion in `deps/egui_term/src/view.rs`. When a cell had the `BOLD` flag, we promoted normal named colors (0-7) to their bright variants (8-15) before calling `get_color()`. This is standard terminal behavior (alacritty_terminal sets the BOLD flag but leaves color promotion to the renderer). Did not fix the visual issue — colors still looked washed out.

**What we know so far:**
- PR #16 (`fix/dim-color-palette`) correctly sets dim colors to normal Catppuccin values to avoid double-dimming (view.rs already applies `linear_multiply(0.7)` for DIM flag)
- The `..Default::default()` in the old palette was pulling in base16 colors for dim variants — that part is confirmed fixed
- Bold→bright promotion alone didn't solve the gray appearance, suggesting the root cause is elsewhere (possibly in how egui renders colors, gamma/sRGB handling, or how the 256-color palette is constructed)
- The black padding on Gemini CLI input box may be a separate issue with how `Named(Black)` (#45475a) vs `Named(Background)` (#1e1e2e) are handled as background colors

**Still needs investigation:** Compare actual RGB values rendered per-cell between Plexi and Ghostty for the same content to isolate whether it's a palette issue, a rendering pipeline issue (sRGB/linear), or something else entirely.

---

## 2026-03-23 — Merged PRs #13 and #14, reviewed PR #16

Merged PR #13 (DejaVu Sans fallback font for Braille/Unicode symbols) and PR #14 (sidebar cursor/X button fixes + link hover improvements). Added per-frame link hover detection so Cmd+hover triggers instantly instead of requiring a mouse move. Also added pointer cursor when Cmd+hovering URLs.

PR #16 (dim color palette fix) is rebased onto main and ready but blocked on the broader color investigation above.

---

## 2026-03-19 — Fix: `clear` content reappearing after zoom/navigate

Root cause: alacritty's `grow_lines()` explicitly pulls scrollback content into the visible area whenever the terminal gains rows. This happens during zoom/navigate — a pane shrinks (tile tree placeholder size), then grows again (zoom overlay size), and old cleared content from scrollback fills the new rows.

Fix in `deps/egui_term/src/backend/mod.rs::resize()`: capture `old_lines` before resize, then call `terminal.grid_mut().clear_history()` if lines grew. Also added `scroll_display(Scroll::Bottom)` after resize to snap viewport on any reflow.

**Known tradeoff:** `clear_history()` nukes ALL scrollback when the terminal grows — not just the lines pulled in. Legitimate scrollback is lost on zoom-in. A future improvement would be to only trim the N lines that `grow_lines` pulled from history, rather than wiping everything.

---

## 2026-03-19 — Repo cleanup: promote egui crate to root, remove legacy code

Removed all legacy codebases (Tauri, Electrobun/Node.js, Playwright tests, npm configs) and promoted `plexi-egui/` to root level. Binary renamed from `plexi-egui` to `plexi`. The `deps/egui_term` path dependency updated accordingly. Icon copied from `src-tauri/icons/icon.png` to `assets/app-icon.png` before deleting `src-tauri/`. Now installable via `cargo install --git`. README rewritten for pure Rust egui architecture.

---

## 2026-03-19 — Remove sidebar minimap

Removed the non-functional minimap section (Map label, node count, visual minimap widget) from the sidebar. It was visual-only clutter with no interactivity. Candidate for future re-implementation as a real feature once pane navigation warrants it.

---

## 2026-03-19 — [FIX] Zoom + tab cycling desync

Tab cycling (Cmd+]/[) while zoomed updated `focused_pane` to the new tab's TileId but left `zoomed_pane` pointing at the old TileId. Result: the dot indicator switched correctly but the overlay kept rendering the old terminal. Unzoom (Cmd+Enter) also failed on first press because the toggle's equality check (`zoomed_pane == Some(focused)`) was comparing two different TileIds. Fix: one conditional in `cycle_tab` — if `zoomed_pane.is_some()`, update it to match the new `focused_pane`. Reinforces the pattern: any code that changes `focused_pane` needs to check whether `zoomed_pane` should follow.

---

## 2026-03-19 — Zoom/maximize pane (Cmd+Enter)

**What:** Cmd+Enter toggles a "zoom" mode that expands the focused pane to fill the central panel with a slight inset (10px), similar to tmux's zoom feature.

**Rendering approach:** Instead of hiding other panes or reparenting, the zoomed pane's slot in the tile tree renders as a dark placeholder (no terminal). After `tree.ui()`, a semi-transparent scrim (black @ 63% opacity) is painted over the entire central panel, then the zoomed terminal is rendered in an inset overlay rect on top. This avoids double-rendering the terminal (which would cause double-input) and keeps the background layout visible but dimmed as a visual cue.

**Auto-unzoom:** Split (Cmd+D/Shift+D), navigate (Cmd+HJKL), and close (Cmd+W) all clear zoom first. Tab cycling (Cmd+]/[) works while zoomed. Context switch inherently changes the active context which has its own `zoomed_pane` field.

**State:** `zoomed_pane: Option<TileId>` on `Context`. Ephemeral — not persisted to workspace file.

---

## 2026-03-19 — [FIX] Focus landing on invisible tab after closing pane

`find_first_pane_in` iterated all children for every container type, including `Tabs`. For a Tabs container, only the active tab is visible, but the function returned whichever child was first in the Vec — often an inactive/hidden tab. This meant after closing the last tab in a pane group, focus could land on a terminal hidden behind another tab. Fixed by checking for `Container::Tabs` and descending only into `tabs.active` instead of iterating all children. One function, ~3 lines added.

---

## 2026-03-19 — Functional contexts (workspaces) with disk persistence

**What:** Contexts in the sidebar are now functional workspaces (like tmux sessions). Each context owns its own tile tree, panes HashMap, and focused pane. Switching contexts swaps the entire view; background terminals keep running. Workspace state persists to `~/.plexi/workspaces/default.json`.

**Architecture decisions:**
- Tree-walking methods (`find_ancestor_tabs`, `find_logical_parent`, `find_pane_in_direction_from`, etc.) moved from `PlexiApp` to `Context` to keep the borrow checker happy — `PlexiApp` methods that need both `self.next_pane_id` and `self.contexts[i].tree` can now call context methods without conflicting borrows.
- `next_pane_id` stays global (on `PlexiApp`) because the PTY event channel is shared across all contexts — pane IDs must be unique globally.
- `close_focused` was restructured into read-only / mutable / cleanup phases to satisfy the borrow checker when accessing `Context` fields.
- Closing the last pane in a context deletes that context (unless it's the only one, then quit). This avoids empty zombie contexts.
- Workspace save uses `egui_tiles::Tree<u64>` serialization directly (serde feature on egui_tiles). On restore, terminals are re-spawned at their saved cwds; stale cwds fall back to context path → home dir.
- Corrupt workspace JSON is renamed to `.backup-{timestamp}.json` and a fresh workspace starts.

**New features:** `+` button creates contexts, double-click renames, hover `x` deletes (2+ contexts), Cmd+1-9 switches contexts, Cmd+Q/exit saves workspace.

**Explicitly deferred:** process persistence (needs daemon), auto-save timer (save-on-exit sufficient for MVP), drag-to-reorder, right-click menus.

---

## 2026-03-19 — [GOTCHA] 60% CPU in debug mode is expected — it's wgpu, not a bug

Investigated high idle CPU usage (~60% in btop). Traced the full repaint chain: eframe 0.31 is already reactive (only repaints on `request_repaint()` / `request_repaint_after()`). The only idle repaint source is cursor blink at ~2 FPS via `request_repaint_after(530ms)`. The 60% is unoptimized wgpu rendering in debug builds — confirmed by running `cargo run --release` which dropped CPU to near-zero. No code fix needed. If debug perf becomes annoying, add `[profile.dev.package."*"] opt-level = 2` to Cargo.toml to optimize deps while keeping app code debuggable.

Also removed a redundant `ctx.send_viewport_cmd(ViewportCommand::Title("Plexi"))` that ran every frame in `update()` — the title was already set once via `ViewportBuilder::with_title("Plexi")` in main.rs.

---

## 2026-03-19 — [FUTURE] Rename binary from plexi-egui to plexi

btop shows the process as "plexi-egui" because `Cargo.toml` has `name = "plexi-egui"`. Defer renaming until the Tauri codebase is removed and `plexi-egui/` becomes the sole binary. Trivial one-liner when the time comes.

---

## 2026-03-19 — [FIX] Cursor rendering: visibility, shape, and unfocused style

Fixed three cursor issues in the forked `egui_term`:

1. **Cursor always visible** — `RenderableContent` never exposed `TermMode::SHOW_CURSOR`, so apps sending `\e[?25l` (hide cursor — used by Claude Code, vim, fzf) still showed a blinking block. Added `cursor_visible` field populated from `terminal.mode().contains(TermMode::SHOW_CURSOR)`.

2. **Unfocused panes drew solid block** — standard terminal behavior (Ghostty, iTerm2, Alacritty) is a hollow 1px outline for unfocused panes. Changed from `RectShape::filled` to `RectShape::stroke` with `StrokeKind::Inside`.

3. **No cursor shape support** — alacritty_terminal tracks `CursorShape` (Block/Beam/Underline/HollowBlock/Hidden) via `term.cursor_style().shape`, but the view always drew a filled block. Added `cursor_shape` field to `RenderableContent` and a `match` in the renderer for Beam (2px vertical line), Underline (2px horizontal line at bottom), and Block (filled rect).

Also fixed text color inversion — was gated on `APP_CURSOR` mode (wrong), now gated on focused + block cursor + cursor visible (correct).

---

## 2026-03-19 — Flat tile tree for equal splits + share equalization on close

**What:** Splitting in the same direction as the parent Linear container now inserts the new pane as a sibling instead of creating a nested container. This keeps the tree flat: three horizontal splits produce three equal thirds, not 50/25/25.

**Key detail — shares on close:** The initial implementation only changed `split_focused` but missed that `close_focused` was manually transferring the closing pane's share to its neighbor (preserving uneven ratios from drag-resizing). Fixed by resetting all sibling shares to `1.0` on close, so remaining panes always redistribute equally.

**Lesson:** Create and destroy paths are coupled. When changing how something is created (split), always read the corresponding teardown (close) in the same pass. The existing share-transfer logic in `close_focused` was the clue that egui_tiles doesn't auto-equalize.

---

## 2026-03-19 — Tab stacking via egui_tiles Tabs containers

**What:** Cmd+T creates a new terminal tab stacked behind the focused pane. Cmd+]/[ cycles between tabs. Replaces Cmd+N (which created a new split alongside root).

**How it works:** `egui_tiles` has a native `Container::Tabs` type. Cmd+T wraps the focused pane + new pane in a Tabs container (or appends to an existing one if focused pane is already in a Tabs container). The tab bar (24px) only appears when a Tabs container has 2+ children — the default `SimplificationOptions::prune_single_child_tabs` auto-removes single-child Tabs containers each frame, so lone panes never show a tab bar.

**Tab bar styling:** Active tab gets terminal bg color (`0x1e1e2e`), inactive tabs get `BG_DARKEST`. Tab titles show "Terminal N" in dim text.

**New tabs inherit cwd** from the focused pane (same as splits).

**Keybindings changed:** Cmd+N removed, Cmd+T added, Cmd+]/[ added for tab cycling.

---

## 2026-03-19 — Post-MVP: tmux-style session persistence

**Deferred until after MVP ships.** Background daemon that owns PTY sessions, GUI connects as a client. Sessions survive GUI restart, processes keep running. This is the #1 differentiator from the UX research but requires an architectural shift (daemon/client split) that touches everything. Validate that people want Plexi first.

---

## 2026-03-19 — TODO: Tauri codebase cleanup / removal

**Deferred.** Once the egui rewrite is feature-complete, remove `src-tauri/`, the Node/npm toolchain, xterm.js, and all Tauri-related config. `plexi-egui/` becomes the sole binary. Benefit is operational: one Rust binary, no webview, no IPC serialization, faster startup, smaller binary.

---

## 2026-03-19 — Keybindings overhaul + app icon + macOS menu FFI (plexi-egui/)

**New keybindings:** Cmd+N (new pane), Cmd+Q (force quit — bypasses close-pane guard via `quitting` flag), Cmd+/ (shortcuts overlay, was Shift+/).

**Cmd+H fix via Cocoa FFI:** macOS intercepts Cmd+H as "Hide Application" before egui/winit see it. Tried three alternatives first:
1. `with_default_menu(false)` — removes entire menu bar, losing Edit (copy/paste) and Window menus. Too aggressive.
2. Alt+HJKL — macOS Option key produces special Unicode chars (∆, ˚, etc.) instead of the base letter, so winit reports the wrong logical key. egui docs explicitly warn against Alt-based shortcuts for this reason.
3. Cmd+[ for left + Cmd+J/K/L for rest — asymmetric and awkward.

**Solution:** Keep default menu, surgically remove "Hide" and "Hide Others" menu items via `objc2-app-kit` FFI in `macos_menu.rs`. Called from `PlexiApp::new()` (after eframe creates the window). Uses `NSApplication::mainMenu()` → first submenu → iterate items → remove those with `hide:` and `hideOtherApplications:` selectors. ~40 lines of safe-ish Rust wrapping unsafe AppKit calls. This is the same approach Ghostty uses.

**App icon:** Embedded via `include_bytes!("../../src-tauri/icons/icon.png")` + `eframe::icon_data::from_png_bytes()` + `ViewportBuilder::with_icon()`. Shows in Dock.

**New pane (Cmd+N):** Creates a fresh terminal (no inherited cwd) and inserts it alongside the root as a horizontal split.

**Dependencies added:** `objc2`, `objc2-app-kit`, `objc2-foundation` (macOS-only, behind `cfg(target_os = "macos")`). These are already transitive deps of winit so no new downloads.

---

## 2026-03-19 — Pane padding color + sizing (plexi-egui/)

Added `TERMINAL_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e)` color constant to match the Catppuccin Mocha terminal background. Updated the pane frame in `tiling.rs` to fill with this color instead of leaving it transparent, so the inner padding inside each pane blends seamlessly with the terminal text area. Increased pane `inner_margin` from 4 to 8 for more breathing room. The outer window margin remains `BG_DARKEST` (darker black) at 4px to match the inter-pane `gap_width`, creating visual consistency around the border.

---

## 2026-03-19 — UX research: competitive patterns + opportunities

**What's working well in the space (patterns worth adopting):**
- cmux's vertical sidebar with per-workspace metadata (branch, ports, notification badges) is the breakout UX pattern — gives spatial context at a glance
- Zellij's stacked panes (collapsed title bars showing what's behind) is the cleanest "tabs behind a pane" visual — avoids the tab-bar clutter problem
- Emerging keybinding consensus: Alt+hjkl or Cmd+hjkl for splits, Cmd+[/] for tab cycling, Cmd+1-9 for workspace jumping
- Fixed sidebar ordering is a must — users cite reordering-by-activity as a top cmux frustration; muscle memory depends on stability
- Activity indicators (dot, badge, color change) on hidden/background tabs are considered essential, not nice-to-have

**cmux pain points = our opportunities:**
1. No process persistence across restart — sessions die on quit; the hardest problem but highest-value differentiator
2. Keybindings not customizable enough — low effort to fix, high user satisfaction payoff
3. Sidebar reorders by activity — actively breaks muscle memory; fixed ordering is a one-liner policy decision

**For MVP:** Don't act on any of this yet. Priority is getting a working multiplexer in front of users. Revisit sidebar metadata and activity indicators once the core split/navigate/close loop is solid.

---

## 2026-03-19 — Uniform spacing + terminal text padding (plexi-egui/)

Changed `gap_width` from `6.0` to `4.0` in `tiling.rs` so inter-pane gaps match the outer `inner_margin: Margin::same(4)` set in `app.rs`. Wrapped both the live terminal and the exited-pane message in `egui::Frame::new().inner_margin(Margin::same(4))` to give text 4px breathing room from pane edges. The focus border in `paint_on_top_of_tile()` operates on the full tile rect (before the frame inset), so it still sits flush at the tile boundary.

---

## 2026-03-18 — Phases 3–4: shell integration + polish (plexi-egui/)

**Shell integration (Phase 3):**
- Forked `egui_term` into `deps/egui_term/` as a path dependency — added `env: HashMap<String, String>` field to `BackendSettings` and wired it into `tty::Options`. Only 3 lines changed in the upstream crate.
- `shell::build_env()` sets TERM, COLORTERM, LANG, LC_ALL, prepends Homebrew PATH on macOS, and injects ZDOTDIR for zsh shell integration.
- `shell::ensure_shell_integration()` writes `.zprofile`/`.zshrc` to `~/.plexi/shell-integration/zsh/` — these source the user's real dotfiles then add a precmd hook emitting OSC 7 (cwd tracking for future split-inherits-cwd).

**Why fork instead of upstream PR:** The egui_term crate is young (v0.1.0) and the maintainer may not want env passthrough in the public API. A local path dep is the lowest-risk approach for MVP. If upstream accepts, we switch back to a version dep.

**Polish (Phase 4):**
- Exited panes show "[process exited]" centered, auto-close on any keypress.
- Window title set to "Plexi" via `ViewportCommand::Title`.
- Removed all `log::info!` debug spam from keys.rs and split_focused.
- Zeroed CentralPanel margins to eliminate padding around terminals.
- Renamed `TerminalPane.id` → `_id` to suppress unused warning.

---

## 2026-03-18 — egui rewrite: pure Rust terminal multiplexer (plexi-egui/)

**Why:** The Tauri + xterm.js architecture has fundamental TUI rendering artifacts (column mismatch, missing glyphs, no synchronized rendering). Native egui rendering via `egui_term` (which wraps `alacritty_terminal`) eliminates all of these. The `egui-poc` branch proved the approach works.

**Architecture:**
- `plexi-egui/` is a standalone Rust crate (sibling to `src-tauri/`, doesn't replace it yet)
- `egui_tiles 0.12.0` for tiled layout with drag-to-resize dividers
- `egui_term 0.1.0` wraps `alacritty_terminal` for PTY + rendering
- No Tokio — egui_term handles PTY I/O on background `std::thread` with `std::sync::mpsc`
- `Tree<PaneId>` stores only u64 IDs; actual `TerminalPane` data lives in a `HashMap`

**Key design decisions:**
- egui_tiles over egui_dock: maintained by Rerun, supports `Linear` containers with H/V splits, `Behavior` trait gives full control (hide tab bars, custom gaps, focus painting)
- Pane type is `u64` (not the full struct) — avoids borrow checker issues since Behavior receives `&mut Pane` but we need separate mutable access to the panes HashMap
- Focus border via `paint_on_top_of_tile()` with `StrokeKind::Inside` to stay within tile bounds
- Window close (`Cmd+W`) intercepted via `close_requested()` + `CancelClose` when multiple panes exist
- Keyboard shortcuts consumed via `ctx.input_mut(|i| i.consume_key(...))` BEFORE `tree.ui()` so terminals don't see them
- Split creates a new Linear container wrapping `[focused, new_tile]`, then replaces focused in its parent — egui_tiles `join_nested_linear_containers` simplification auto-flattens same-direction nesting

**Deferred to Phase 3 (requires egui_term fork):**
- `BackendSettings` has no `env` field — can't inject ZDOTDIR, LANG, COLORTERM, PATH/Homebrew. Need 3-line fork to wire env HashMap into alacritty_terminal's `tty::Options`.

---

## 2026-03-18 — E2E binary testing with tauri-plugin-webdriver

**Problem:** The official `tauri-driver` does not work on macOS — it prints "not supported on this platform" because Apple provides no WKWebView WebDriver tool. The existing Playwright tests run against a static HTTP server (mock backend, no real PTY sessions).

**Solution:** Community crate `tauri-plugin-webdriver` (Choochmeque) embeds a W3C WebDriver server inside debug builds. A companion `tauri-webdriver` CLI on port 4444 launches the `.app` binary and proxies WebDriver commands. WebdriverIO connects as the test client.

**Setup:**
- `tauri-plugin-webdriver` added as optional dep behind `webdriver` Cargo feature
- Plugin registered in `lib.rs` with `#[cfg(feature = "webdriver")]`
- Build: `cargo build --features webdriver` (or `npm run test:e2e:binary:build`)
- Run: `npm run test:e2e:binary`

**Key gotchas discovered:**
1. `browser.execute()` serializes `undefined` args as `null`, which bypasses JS default parameter values. Workaround: branch on whether the arg is defined before calling execute.
2. xterm.js with WebGL addon renders to `<canvas>`, not `.xterm-rows` divs — DOM text queries on `.xterm-rows` return empty. Use `__PLEXI_DEBUG__.getPanelBuffer()` instead.
3. `Cmd+N` / `Cmd+W` are native menu accelerators handled by macOS, not DOM key events. WebDriver can't trigger them. Use `__PLEXI_DEBUG__.runCommand()` to invoke app commands.
4. PTY sessions need ~1s after `openSession` before the shell prompt arrives. Tests must `waitForPtyReady()` before sending input.
5. No headless mode on macOS — WKWebView requires a window server. On Linux CI, Xvfb provides a virtual display.

**Alternatives evaluated and rejected:**
- `tauri-driver` (official): macOS not supported
- Appium mac2: can't access WKWebView DOM
- Playwright WebKit: can't connect to WKWebView in native apps
- Computer Use / AI vision: non-deterministic, expensive, no DOM assertions
- `danielraffel/tauri-webdriver`: similar approach but macOS-only, 3 open bugs, stale

**Test coverage (17 tests, ~25s):**
- App shell: title, sidebar/workspace render, context list, clean state
- Terminal lifecycle: open with real PTY, execute command + verify output
- Splits: split-right, close-keeps-original, split-down
- Top-level nodes: new-node-right, new-node-down
- Ephemeral directory: creates temp dir under `~/.plexi/`, cd's into it, splits pane and verifies cwd propagation via OSC 7, creates a file in one pane and reads it from the sibling, tears down temp dir (with `after()` safety net for failed runs)
- Cleanup: close all panels

**TODO:**
- Add keyboard shortcut for `new-context` (currently only accessible via sidebar button / modal)
- Context creation test needs modal automation or a programmatic API

---

## 2026-03-18 — Future enhancement: Claude Code notification routing + conversation cycling

**Feature idea:** Surface Claude Code conversations/notifications in the Plexi UI so you can cycle through multiple sessions waiting for input (e.g., "5 chats need responses, hop between them").

**How cmux does it:** Uses a hook injection system. It wraps Claude Code with environment variables pointing to hook commands (`CMUX_ON_NOTIFICATION`, `CMUX_ON_WAITING_FOR_INPUT`, etc.). When Claude Code hits lifecycle events, it executes the hooks, which fire back to cmux via socket API with structured metadata (status, notification text, waiting_for_input flag).

**Options for Plexi (in priority order):**

1. **Request hook support from Anthropic** (preferred, Option A): File a feature request with Claude Code team to support `PLEXI_ON_*` environment variables. If Claude Code adopts this, Plexi can inject them when spawning sessions and get structured notifications via IPC callback.

2. **Parse OSC sequences Claude Code already emits** (Option B): Check if Claude Code emits OSC 777 (desktop notification) or OSC 9/99 (status). If so, parse them from PTY output like OSC 7 (cwd tracking). Less structured than hooks but works today.

3. **Implement hook system yourself** (Option C, medium effort): Patch or wrap Claude Code to inject Plexi's own hook environment variables. Hooks call back to Tauri backend via IPC. Full control but requires maintaining a Claude Code wrapper.

**MVP approach:** Defer until users ask for it. If this becomes a priority, start with Option A (upstream request) or Option B (parse existing sequences). Option C is a fallback.

**References:** cmux architecture at [manaflow-ai/cmux](https://github.com/manaflow-ai/cmux) PR #1306.

---

## 2026-03-18 — TUI rendering: root cause analysis + libghostty evaluated (deferred)

**Why Plexi is janky with TUIs (Claude Code, htop, lazygit, etc.):**

xterm.js measures cell size *backward*: render HTML → measure DOM element → derive cell dimensions → set PTY size. Native terminals (Ghostty, iTerm2) go the other way: read OS font metrics → derive cell dimensions → render. Any browser rounding or CSS approximation in the xterm.js path compounds into a PTY col count that doesn't match what's actually displayed. TUI apps query `TIOCGWINSZ`, get the wrong number, and wrap/overlap content.

Specific xterm.js failure modes:
- **FitAddon col math**: documented upstream; approximates scrollbar width rather than measuring it
- **Unicode width tables**: shipped tables are ~2019 vintage — newer emoji are 1-cell in xterm.js but 2-cell in the PTY. This was the immediate autocomplete bug (emoji in completion entries pushed cursor wrong)
- **No synchronized rendering** (ANSI 2026): Ghostty supports batched frame commits to eliminate partial-render flicker; xterm.js doesn't
- **No Kitty Keyboard Protocol**: modern TUIs increasingly rely on this for reliable modifier+key combos

**libghostty evaluated and rejected for now:**

libghostty would fix the rendering accuracy (it uses OS font metrics → Metal on macOS), but it cannot be embedded in a Tauri app:
- Its rendering layer expects direct Metal/OpenGL GPU surface access — it renders into a native AppKit/GTK view, not an offscreen buffer you can composite into a WebView
- The apps that have embedded it (cmux, mdnb, pynb) are all native Swift/AppKit — cmux's creators explicitly rejected Tauri/Electron for this reason
- Unstable C API (officially marked in-progress; stable release targeted sometime 2026), requires Zig toolchain, no pre-built binaries

**Decision:** Accept xterm.js limitations for the MVP. Simple shell usage works fine; TUI-heavy apps suffer. If TUI quality becomes a core differentiator (e.g., "the terminal for Claude Code users"), the right long-term path is a native rendering layer — either a native AppKit view overlay in Tauri, or rebuilding the terminal component entirely outside the WebView. Defer until there are real users to justify the effort.

**Deferred fixes to revisit when needed:**
1. Patch the acute emoji width bug: force double-width emoji in xterm.js via a custom `unicodeService` override
2. Replace fitAddon column calc: measure cell size from canvas `measureText()` on the actual font instead of the DOM probe span
3. Monitor libghostty C API stability (aimed for late 2026 stable) — revisit embedding feasibility then

---

## 2026-03-18 — TUI rendering artifacts: UNSOLVED — known limitation

**Status:** Reverted all attempted fixes. The column-count safety margin, CSS specificity fix, and timing fix were all insufficient — Claude Code's Ink-based TUI still renders with garbled re-renders, missing icons (◆ rendered as `???`), and text overlap.

**What we know:**
- The issue is a column-count mismatch between what xterm.js fitAddon reports to the PTY and what the WebGL renderer actually displays
- Native terminals (Ghostty, iTerm2) don't have this because their renderer and column math are the same code path — xterm.js has an inherent measurement gap between fitAddon (CSS pixels) and the WebGL renderer
- The missing diamond icons (`◆` → `???`) are a separate issue — likely a font/glyph coverage problem in the WebGL renderer's texture atlas
- Multiple fix attempts (safety margin subtraction, CSS scrollbar specificity, fit timing) failed to fully resolve it

**Attempted fixes (all reverted):**
1. Subtracting 1 column after fitAddon.fit() — still garbled
2. Fixing CSS specificity on scrollbar width (6px override) — no visible improvement
3. Synchronous fit + rAF re-fit after WebGL addon load — no visible improvement

**This is a known class of xterm.js issues.** TUI-heavy apps (Claude Code, htop, etc.) are affected. Simple shell usage works fine. Needs deeper investigation — possibly a custom fitAddon that reads dimensions directly from the active renderer, or disabling WebGL for affected sessions.

---

## 2026-03-17 — TUI rendering artifacts in xterm.js (Claude Code, Ink apps) — OPEN

**Symptom:** Claude Code (and likely other Ink/TUI apps) renders with column-alignment artifacts inside Plexi. Specific issues observed:
- Two `◆◆` glyphs in the separator line appear and disappear as the window is resized — confirmed to be a wrapping/column-width issue, not a missing font glyph issue
- Right-panel header content shows a `m]` prefix (truncated label, visible as wrap artifact)
- Bottom status bar sections overlap or concatenate without proper spacing
- Text content from one logical row bleeds onto the next visual row

**Key observation:** The `◆` glyphs in the separator line become MORE numerous when the window is narrower and FEWER when wider — they are real rendered glyphs, but wrapping causes them to spill onto adjacent lines, implying the PTY is reporting MORE columns than xterm.js is actually displaying.

**Root cause hypothesis (unconfirmed):** The PTY col count and xterm.js display col count are mismatched. Likely causes:
1. The fitAddon subtracts scrollbar width incorrectly (see CSS below)
2. The `overviewRuler: { width: 1 }` option may not map correctly in some xterm.js 6 paths
3. CSS specificity conflict: `.scrollbar.scrollbar.vertical { width: 6px !important }` overrides `.scrollbar.vertical { width: 0 !important }` due to higher specificity — the scrollbar may be taking 6px of layout space while fitAddon only subtracts 1px (the ruler width), creating a ~5px discrepancy

**What was tried and ruled out:**
- Adding `"Apple Color Emoji"` and `"Apple Symbols"` to the font-family fallback → made column alignment WORSE (emoji font metrics interfere with xterm.js char-width calculations). Reverted.
- Adding `@xterm/addon-unicode11` and activating it before `terminal.open()` → PARTIAL FIX. Eliminated the garbled full-layout issues (misaligned text across the whole terminal). The major rendering is now correct. The remaining `◆◆` and alignment issues persist. **This fix is in place and correct — do not revert.**
- Moving `ensurePanelSessions()` to after `syncVisiblePaneRuntimes()` + synchronous `fitAddon.fit()` before `terminal.open()` → no visible improvement. Reverted. The PTY size mismatch hypothesis (PTY spawning at 80×24) was not the primary cause since Claude Code receives SIGWINCH and redraws.

**Current state (after unicode11 fix, emoji fonts reverted, timing revert):** Most of the layout is correct. The remaining issue is a consistent column-count discrepancy between PTY and xterm.js display, causing TUI apps that use the full terminal width to overflow by ~2–5 cols and wrap content onto the next line.

**Next steps to investigate:**
- Audit CSS scrollbar rules for specificity conflicts — the 6px `.scrollbar.scrollbar.vertical` override may be the culprit
- Add a diagnostic: run `tput cols` in a Plexi session and compare to `window.innerWidth` / observed char count to confirm the actual discrepancy
- Consider whether `overviewRuler: { width: 1 }` in TERMINAL_PROFILE is correctly recognized by xterm.js 6 (vs the older `overviewRulerWidth` flat option)
- The fitAddon source reads: `t = scrollback === 0 ? 0 : overviewRuler?.width || 14` — if `overviewRuler` is not stored in `terminal.options`, t defaults to 14, causing fitAddon to under-report cols by ~1

---

## 2026-03-17 — Switch xterm.js to WebGL renderer for better color fidelity

Added `@xterm/addon-webgl` and activated it after `terminal.open()` in `xterm-runtime.js`. Fixes wrong colorization in TUI apps (Claude Code, etc.) vs Ghostty. The default Canvas 2D renderer was the culprit — it's less accurate than a GPU-composited path.

Includes an `onContextLoss` handler that disposes the WebGL addon if the GPU context is lost (can happen when window backgrounds on macOS), falling back to canvas automatically. Without this handler, a context loss leaves the terminal blank permanently.

Vendor script added at `vendor/xterm/addon-webgl.js`; `copy-vendor` script updated to include it.

---

## 2026-03-17 — Fix Cmd+V paste showing permission popup instead of pasting

Pressing Cmd+V in the terminal showed a WebView permission popup ("Paste from clipboard?") at the cursor instead of cleanly pasting text.

**Root cause:** The `paste_from_clipboard` keybind handler in `app.js` was intercepting Cmd+V, calling `event.preventDefault()`, then manually reading the clipboard via `navigator.clipboard.readText()`. In Tauri's WKWebView on macOS, that API triggers a native clipboard permission dialog.

**Fix:** Removed the manual clipboard read. The keybind handler now returns `true` to let the keypress pass through to xterm.js, which has its own built-in `paste` event listener. The browser fires the native `paste` event (no permission needed), xterm.js picks it up, and routes the text through `onData` into the PTY session.

**Dead end:** Tried using `tauri-plugin-clipboard-manager` to bypass the WebView permission system via native OS clipboard access. Plugin compiled and registered fine, but the invoke calls silently failed — paste did nothing at all. Reverted. The xterm.js native paste path is simpler and requires zero Rust changes.

**Lesson:** Don't fight the browser's clipboard security model — use the native `paste` event flow instead of `navigator.clipboard.readText()`. xterm.js already handles this correctly if you let the key event through.

---

## 2026-03-17 — Custom title bar and window dragging with titleBarStyle Overlay

Switched from default macOS gray title bar to a transparent overlay bar (`"titleBarStyle": "Overlay"`, `"hiddenTitle": true` in `tauri.conf.json`) so the app background color extends into the title bar area. Bumped `--window-top-inset` from `6px` to `28px` for macOS so content clears the traffic light buttons.

**Window dragging:** `data-tauri-drag-region` on the toolbar elements wasn't enough — the attribute only applies to the exact element it's on, not children, so child `div`s and `span`s inside the toolbar swallow the mousedown before it reaches the drag region. Fixed by adding a `mousedown` listener that calls `getCurrentWindow().startDragging()` when the click target isn't an interactive element.

**Critical:** `startDragging()` requires the capability permission `core:window:allow-start-dragging` in `src-tauri/capabilities/default.json`. Without it, the call silently fails — no error, no drag. This is a Tauri 2.x security sandbox requirement.

## Future: Single-instance enforcement

By default Tauri does not prevent multiple app instances from running simultaneously. A second launch opens a second process with its own config read/write cycle — potential for concurrent writes to `~/.plexi/`. Not an issue now (no users, macOS Dock typically re-focuses the existing window anyway). When it matters, add the official [`tauri-plugin-single-instance`](https://v2.tauri.app/plugin/single-instance/).

---

## Future: E2E test suite with tauri-driver

Full end-to-end tests using `tauri-driver` + WebdriverIO against a compiled binary. Spin up a clean, unconfigured app (no `~/.plexi` state) and exercise every major user flow:

- Create a new terminal session, run a command, verify output appears
- Split panes horizontally and vertically
- Close a pane, verify others are unaffected
- Workspace save + restore (relaunch app, verify layout and sessions recover)
- Resize terminal, verify PTY SIGWINCH propagates correctly

This is the right long-term confidence net before releases. Not MVP — defer until the core feature set stabilizes and there are real users to break things. When implementing, start with the official Tauri guide: https://tauri.app/develop/tests/webdriver/

---

## 2026-03-17 — Shell integration via ZDOTDIR injection for cwd tracking

Split terminals and workspace saves were always showing the initial session directory (e.g. `~`) instead of the user's current directory. `panel.cwd` was only set once at session spawn and never updated because the shell wasn't emitting any cwd signal.

**Fix:** ZDOTDIR injection — the same approach used by Ghostty, iTerm2, and WezTerm.
- `shell_integration.rs` writes `~/.plexi/shell-integration/zsh/{.zshrc,.zprofile}` at startup (idempotent)
- The `.zshrc` sources the user's real `~/.zshrc` (via `PLEXI_ORIG_ZDOTDIR`), then appends a `precmd` hook
- The hook emits **OSC 7** (`\e]7;file://hostname/path\a`) — the standard cwd protocol
- Replaced the custom `PlexiCwd` OSC 633 sequence with OSC 7 in `session-output.js`, mock bridge, and tests

**Why OSC 7 over the custom PlexiCwd sequence:** OSC 7 is already supported by fish (built-in), and shell integration scripts for bash/fish are widely available. fish users already get cwd tracking for free. Bash support just needs an additional `shell_integration.rs` script later.

**Also fixed:** `home_dir` is now returned from `SessionStartedMessage` so the frontend initializes `homeDirectory` immediately (fixes `cwdLabel` showing full paths instead of `~` in workspace saves).

**Zsh only for now** — bash/fish integration scripts are the next step when needed.

## 2026-03-17 — Double input bug in production Tauri builds (RESOLVED)

**Status: fixed**

First keystroke after each prompt appeared doubled in production builds (`tauri build`), but worked perfectly in dev mode (`tauri dev`). Same bug existed in the earlier Electrobun version. Typing "echo hi" rendered as "ececho hi".

**Root cause:** Missing locale environment + non-login shell. When a macOS app launches from `/Applications` (via Finder/launchd), it gets a barebones environment — no `LANG`, no `LC_ALL`. In dev mode, `tauri dev` inherits the full terminal environment, so everything works. Without `LANG=en_US.UTF-8`, zsh's ZLE and plugins (autosuggestions, syntax highlighting, Starship) miscalculate character widths on the first keystroke, position the cursor wrong, and the first character renders with ghost artifacts.

**Fix (pty.rs):**
```rust
Command::new(shell_path)
    .arg("-l")  // login shell — sources ~/.zprofile, /etc/zprofile
    .env("LANG", "en_US.UTF-8")
    .env("LC_ALL", "en_US.UTF-8")
```

**What we ruled out first (all dead ends):**
- Custom native menu / `Menu::default` — no effect
- Menu event listener in JS — no effect
- Ghost processes — none found
- Doubled IPC calls — debug logs showed input fires once, output seq numbers are clean
- xterm.js `attachCustomKeyEventHandler` workaround intercepting all printable chars — no effect
- Recent code regression — bug existed in older commits too (`b190e64`, `c761d23`)

**Lesson:** When spawning PTY shells from a GUI app on macOS, ALWAYS set locale env vars and spawn as a login shell. The launchd environment is not the same as a terminal environment. This applies to any framework (Tauri, Electrobun, Electron).

## 2026-03-17 — Implement ~/.plexi directory: workspace persistence + config file

Added filesystem persistence for workspaces and a global config file. Structure:

```
~/.plexi/
  config.json          # global settings (terminal, shell, keyboard)
  workspaces/
    default.json       # workspace layout + contexts + panel metadata
    <name>.json        # future: multiple named workspaces
```

**Key decisions:**

1. **Workspaces are named files, not a single workspace.json.** Each workspace is `~/.plexi/workspaces/<name>.json`. Currently only "default" is used, but the API supports multiple named workspaces for future workspace switching.

2. **Config overrides in workspace files.** Workspace documents already serialize `terminal` and `keyboard` keys. These can override the global config via `resolveConfig()` in `plexi-config.js`. No new format needed.

3. **Config file written on first launch.** If `~/.plexi/config.json` doesn't exist, defaults are written from `plexi-config.js`. Values come from the existing hardcoded constants in `app-constants.js`. Comments in the code note which settings aren't actually wired up yet (theme, fonts, keybinds).

4. **localStorage kept as fallback.** Every save still writes to localStorage in addition to disk. This means the app degrades gracefully if the disk write fails.

5. **Skipped "profiles" concept.** Profiles would bundle config + workspace together — unnecessary complexity until users ask for it.

6. **Rust side uses `dirs` crate** for `home_dir()`. Workspace names are sanitized to prevent path traversal.

**New files:** `src-tauri/src/config.rs`, `src/mainview/plexi-config.js`
**Modified:** `lib.rs` (6 new commands), `tauri-session-bridge.js` (bridge stubs → real IPC), `workspace-storage.js` (tauri mode support), `app.js` (config loading + mode checks)

## 2026-03-17 — Future enhancement: scriptable workspace layouts
Like tmuxinator/tmuxp — user-defined named layouts that open split panes with specific commands pre-launched (e.g. "dev stack" = frontend + backend side-by-side). First-class differentiator for Plexi. Not MVP — shelved until there are users.

## 2026-03-17 — Real PTY sessions fixed on macOS Tauri

**Status: resolved**

The actual root cause of `[session failed] undefined` was not the frontend retry loop. It was the PTY backend.

- `pty-process 0.4` was being used with the older borrowed-PTS spawn API. On macOS this fails during controlling-terminal setup with `Inappropriate ioctl for device (os error 25)`, so `open_session` rejected before the shell ever started.
- The frontend then rendered `error.message`, but Tauri invoke errors can arrive as plain strings/objects, so the user-facing result became `undefined` instead of the real backend error.

**Fixes applied:**

1. Upgraded `pty-process` from `0.4` to `0.5.3`.
2. Switched PTY creation to `blocking::open()` and moved the slave PTY into `Command::spawn(...)` using the current API, which works on macOS.
3. `spawn_shell()` now returns the resolved working directory so the frontend gets a real `cwd` immediately.
4. Tauri bridge errors are normalized to real `Error` objects before surfacing to the UI.
5. Added a Rust session test that opens a real shell, sends `printf '__PLEXI_OK__\n'`, and verifies the output round-trip.
6. Native `npm run dev` smoke check now shows successful session creation in logs:
   - `Spawned shell: /bin/zsh (80x24)`
   - `Opened session panel-1 with shell zsh (80x24)`

**Additional Tauri architecture issue found:**

- `beforeDevCommand` used `npx serve src -l 1415`, and if port `1415` was busy it silently picked a random port while Tauri still loaded `http://localhost:1415/mainview/`. That creates stale-frontend debugging traps. Replaced it with `python3 -m http.server 1415 --bind 127.0.0.1 --directory src` so port conflicts fail loudly instead of drifting.

## 2026-03-17 — Real PTY sessions not opening: current blocker

**Status: unresolved — handing off**

The Tauri IPC bridge is now wired up and `window.__TAURI_INTERNALS__` is detected correctly, so the app is no longer falling back to the mock shell. However, real zsh sessions are still not starting successfully. Symptoms:

- UI shows `[session failed] <error>` in the terminal panel
- `poll_session_output` floods the console with "Session not found" (hundreds of times before stopping)

**What was fixed in this session:**

1. **`window.__TAURI__` not injected**: `withGlobalTauri: true` added to `tauri.conf.json` under `app`. Without it, `window.__TAURI__` is undefined and the bridge falls back to mock every time.

2. **Wrong detection check**: `hasTauriRuntime()` was checking `window.__TAURI__.invoke` (Tauri 1.x location) but Tauri 2.x puts it at `window.__TAURI__.core.invoke`. Fixed to use `window.__TAURI_INTERNALS__` for detection (always injected by Tauri regardless of `withGlobalTauri`) and `getInvoke()` helper that tries `__TAURI__.core.invoke` then falls back to `__TAURI_INTERNALS__.invoke`.

3. **PTY spawn with bad CWD**: Workspace restored from localStorage had `cwd: "/mock/project"` (from old mock sessions). `pty.spawn_shell()` with a non-existent CWD fails. Fixed in `pty.rs` to silently fall back to `$HOME` if the saved CWD path doesn't exist.

4. **Infinite retry loop on session failure**: `ensurePanelSession` was called on every `render()`. When `openSession` threw, it called `panelSessions.delete(panel.id)`, which allowed the next render to retry immediately — infinite loop. Also called `render()` from inside the catch block, making it worse. Fixed by adding a `panelSessionFailed` Set; failed sessions are not retried until explicitly closed.

5. **Polling loop on session not found**: `_startPolling` caught errors with `console.error` but never stopped the interval. 1000+ "Session not found" errors per run. Fixed: stop polling after 3 consecutive errors.

6. **`just dev-fresh`**: Added `justfile` with `dev` and `dev-fresh` recipes. `dev-fresh` uses `tauri dev --config` to override `devUrl` to `src/fresh.html`, which clears `localStorage["plexi.workspace.v2"]` before redirecting to `/mainview/`. Eliminates stale mock-era workspace state on startup.

**Current state / what the next agent should investigate:**

After all the above fixes, `just dev-fresh` + `Cmd+N` still shows `[session failed]` and "Session not found" errors (though now only ~13 instead of 1000+). The root cause is not yet confirmed. Key things to check:

- **What is the actual error message from `open_session`?** Add `console.error("openSession failed:", error)` to the catch block in `ensurePanelSession` in `app.js` and check DevTools console. The error string from Rust will say whether it's "Failed to spawn PTY: ..." or "Session already exists" or something else.
- **Is `open_session` even being called?** Add a `console.log` before the `invoke("open_session", ...)` call in `tauri-session-bridge.js` to confirm IPC is reaching Rust.
- **Is the Tauri app being fully rebuilt?** Changes to `pty.rs` require a full Rust rebuild. `npm run dev` triggers this, but `just dev` may not if the Tauri watcher doesn't detect the change. Confirm with `cargo build` directly.
- **Check Tauri logs**: Run `RUST_LOG=debug npm run dev` or look at `~/Library/Logs/dev.plexi/` for PTY spawn errors.
- **The remaining 13 "Session not found" errors**: These come AFTER the polling stop-on-3-errors fix. 13 / 3 = ~4 separate polling intervals were started, meaning `open_session` succeeded for ~4 sessions before they disappeared. This suggests sessions ARE being opened (Rust side OK) but then something calls `close_session` or removes them. Possible culprit: `syncVisiblePaneRuntimes` disposes runtimes on re-render, but does NOT call `closePanelSession` — check whether `disposePaneRuntime` is inadvertently triggering session cleanup.

## 2026-03-17 — Fix Tauri app initialization and IPC bridge

Multiple issues prevented the Tauri rebuild from being functional:

1. **Electrobun bare import crash**: `session-bridge.js` had `import { Electroview } from "electrobun/view"` — a bare specifier that crashes in any non-Electrobun environment (Tauri, browser). `app.js` imported both bridges unconditionally, so this killed the entire module graph. Fix: removed Electrobun bridge import from `app.js`; `tauri-session-bridge.js` now falls back to mock bridge directly.

2. **Double log plugin registration**: `lib.rs` had `.plugin(tauri_plugin_log::...)` on the builder AND again inside `.setup()`. Also had two `.setup()` blocks. Consolidated to one empty `.setup()`.

3. **IPC parameter naming**: Tauri 2.x auto-converts camelCase JS args → snake_case Rust params. Original bridge used `panel_id` (snake_case) in JS which wouldn't match. Fixed all IPC calls to use camelCase (`panelId`, `lastSeq`, etc.).

4. **Blocking PTY reads under mutex**: `poll_session_output` locked the SessionManager mutex then did a blocking `read()` on the PTY fd. If no data, this blocked all other IPC commands. Fix: set PTY fd to `O_NONBLOCK` via `libc::fcntl` after spawn.

5. **Polling never started after openSession**: `openSession()` fired `onStarted` but never called `_startPolling()`. Terminal output never arrived. Fix: start polling immediately after successful open.

6. **Dev server for Playwright**: Added `beforeDevCommand` with `npx serve src` to `tauri.conf.json` so Tauri dev mode serves frontend over HTTP. Playwright tests now point to `/mainview/` path. All 10 e2e tests pass.

## 2026-03-15 — Fix 14px black gap on right side of xterm terminal

xterm's FitAddon (v6) subtracts a scrollbar width when `scrollback > 0`: `overviewRuler?.width || 14`. With no `overviewRuler` option set, it always subtracts 14px, leaving a black gap where the canvas doesn't reach the terminal frame edge.

Fix: set `overviewRuler: { width: 1 }` in Terminal options so FitAddon subtracts 1px instead of 14px. Then hide the resulting 1px ruler canvas (`.xterm-decoration-overview-ruler`) and the native scrollbar element (`.scrollbar.vertical`) with CSS `display: none / width: 0`. Also suppress the native viewport scrollbar with `scrollbar-width: none`.

Setting `overviewRuler: { width: 0 }` doesn't work because `0 || 14 = 14` — needs a truthy value to bypass the fallback.

## 2026-03-14 — Remove overview mode entirely

Deleted the overview feature: `#overview-shell` HTML, all `.overview-*` CSS, `mode`/`camera` state, `toggleMode`/`panCamera`/`adjustZoom`/`resetViewport` from workspace-state.js, `toggleOverview`/`zoomIn`/`zoomOut` commands, all keyboard handlers, and `renderOverview`/`renderOverviewHud` functions.

Why: Overview was decorative at this stage — no dragging, no meaningful spatial navigation beyond what the minimap already provides. The mode boundary was leaky (zoom changed terminal font size even in overview mode). An empty overview state duplicated the empty landing screen. Cut it until there's a real use case.

Also fixed two pre-existing gaps exposed by the test suite: `#focus-title` was showing directory name instead of panel title, and context rename was using a custom modal instead of `window.prompt()`. Simplified rename to native prompt. Added `#toolbar-context` and `#focus-position` to the toolbar (were already tested, just missing from HTML).

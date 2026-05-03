# Changelog

Newest releases appear first.

## [3.4.48] — 2026-05-03

### Changes
- - docs: DEV_LOG PR #580 + platform behavior validation lesson - feat(macos): show version in menu bar for non-stable builds (#580)
- docs: DEV_LOG PR #580 + platform behavior validation lesson
- feat(macos): show version in menu bar for non-stable builds (#580)
- - docs: DEV_LOG PR #575 context naming modal when sidebar hidden - docs: add contact section to README - docs: fix DEV_LOG issue reference #580 → #582 - docs: clarify mit priority label — sits atop P1-P4, usually paired with P1
- docs: DEV_LOG PR #575 context naming modal when sidebar hidden
- docs: add contact section to README
- docs: fix DEV_LOG issue reference #580 → #582
- docs: clarify mit priority label — sits atop P1-P4, usually paired with P1
- - docs: add mit label to issue priority scheme in CLAUDE.md - docs: DEV_LOG PR #581 drop-event breadcrumbs in zoomed overlay - feat(logging): add drop-event breadcrumbs in zoomed overlay path (#581)
- docs: add mit label to issue priority scheme in CLAUDE.md
- docs: DEV_LOG PR #581 drop-event breadcrumbs in zoomed overlay
- feat(sidebar): show context naming modal when sidebar is hidden (#575)
- feat(logging): add drop-event breadcrumbs in zoomed overlay path (#581)
- docs: add lesson about uncommitted bump on alpha
- chore: DEV_LOG PR #566 first-launch CLI setup
- docs: add lesson about uncommitted bump on alpha
- feat(install): first-launch CLI setup — symlink plexi to PATH (#566)
- chore: bump alpha to 3.4.43
- chore: DEV_LOG PR #576 ghost empty state fix
- chore: add ET timestamps to all changelog entries; emit time in bump-alpha (#574)
- chore: DEV_LOG PR #574 changelog ET timestamps
- fix(ui): make ghost empty state impossible — defensive tree.root guards + clear zoomed_pane on app close (#576)
- chore: add ET timestamps to all changelog entries; emit time in bump-alpha (#574)
- chore: bump alpha to 3.4.41
- feat(widgets): dismissable_modal helper — escape + click-outside for overlays (#570)
- chore: DEV_LOG PR #570 dismissable_modal helper
- feat(widgets): dismissable_modal helper — escape + click-outside for overlays (#570)
- chore: bump alpha to 3.4.40
- chore: DEV_LOG PR #567 file picker capability
- fix(terminal): explicitly push first char before iter_from in selectable_content (#569)
- chore: DEV_LOG PR #569 terminal copy first-char fix
- feat(pgap): file picker capability — OpenFilePicker/FilePicked/FilePickCancelled (#514) (#567)
- fix(terminal): explicitly push first char before iter_from in selectable_content (#569)
- feat(pty): inject PLEXI_PANE_ID + PLEXI_SOCKET into every managed PTY environment (#565)
- chore: DEV_LOG PR #565 PTY env injection
- feat(pty): inject PLEXI_PANE_ID + PLEXI_SOCKET into every managed PTY environment (#565)
- chore: bump alpha to 3.4.37
- chore: DEV_LOG PR #564 HostHarness
- feat(testing): HostHarness — headless egui test harness (#552) (#564)
- feat(welcome): add Plexi logo + centered wordmark (#562)
- chore: DEV_LOG PR #562 welcome logo + wordmark
- feat(welcome): add Plexi logo + centered wordmark (#562)
- chore: bump alpha to 3.4.34
- chore: DEV_LOG PR #560 welcome screen email
- feat(welcome): add email mailto: link to welcome screen (#560)
- feat: add just pr-install and pr-clean for PR testing flow (#559)
- - dev_log: PR #556 sidebar hit-rect fix - fix(sidebar): stabilise hit rects + clear renaming_window on reorder (#556) - docs: add Failed PR reset protocol to CLAUDE.md - Add project-level triage skill; unignore .claude/ directory - docs: replace ship cycle with /ship skill reference, add testing label
- dev_log: PR #556 sidebar hit-rect fix
- fix(sidebar): stabilise hit rects + clear renaming_window on reorder (#556)
- docs: add Failed PR reset protocol to CLAUDE.md
- Add project-level triage skill; unignore .claude/ directory
- docs: replace ship cycle with /ship skill reference, add testing label
- - dev_log: PR #551 tool registration + token diagnostics - docs: add in-progress label to feature branch and ship cycle workflows
- dev_log: PR #551 tool registration + token diagnostics
- docs: add in-progress label to feature branch and ship cycle workflows
- fix(ai): tool registration + token count diagnostics (#546) (#551)
- Add protocol scheme to OpenRouter HTTP-Referer header
- Improve bump message generation; simplify OpenRouter Referer header
- chore: bump alpha to 3.4.29, update changelog
- Improve text input widget clamping and chat bubble sizing against pane bounds
- chore: bump alpha to 3.4.28, update changelog
- Fetch complete OpenRouter generation metrics; fix text input widget clamping to pane bounds
- chore: bump alpha to 3.4.27, update changelog
- chore: bump alpha to 3.4.26, update changelog
- chore: bump alpha to 3.4.25, update changelog
- chore: add DEV_LOG entry for PR #549 (chat UI improvements)
- chore: bump alpha to 3.4.24, update changelog
- chore: update ship cycle to commit DEV_LOG before bump-and-install
- chore: bump alpha to 3.4.23, update changelog
- feat: markdown chat bubbles, TextInput scroll, counter shortcuts (#549)
- feat(parallax): MVP editor app + SDK ctx.image() (#548)
- chore: bump alpha to 3.4.22, update changelog
- fix: make @app.tool decorator cumulative + include usage in OpenRouter streams
- chore: bump alpha to 3.4.21, update changelog
- fix: update OpenRouter HTTP-Referer to plexiapp.com
- chore: add DEV_LOG entry for PR #544 (chat UI, TextInput refocus fix)
- chore: bump alpha to 3.4.20, update changelog
- feat(chat): polished bubble UI + TextInput refocus fix (#544)
- feat(input-inspector): add inputs page and per-category event filtering
- fix(logging): add info-level broker tool log, fix [log] section header in config
- chore: rename bundle to Plexi Alpha, default chat tier to low
- docs: update DEV_LOG with v3.7 context injection and TextInput fixes
- chore: bump alpha to 3.4.19, update changelog
- feat(v3.7): context injection for all open panes, fix text cutoff and TextInput focus
- docs: consolidate project docs — remove ARCHITECTURE.md and ROADMAP.md, add GLOSSARY.md, update CLAUDE.md
- chore: bump alpha to 3.4.18, update changelog
- fix(examples): log scroll events in input-inspector
- chore: bump alpha to 3.4.17, update changelog
- feat(host): add TextRow draw command with host-measured text layout
- chore: bump alpha to 3.4.16, update changelog
- DEV_LOG: log PR #540 text_row() layout primitive
- feat(sdk): add text_row() host-measured text layout primitive (#540)
- feat(examples): input-inspector POC for issue #331 (#529)
- chore: promote to beta — v3.4.14
- fix(promote.sh): auto-push unpushed commits instead of failing
- chore: bump alpha to 3.4.13, update changelog
- fix(justfile): bump-alpha SIGPIPE with pipefail — use git log -1 not | head -1 (#537)
- fix(routing): AiQuery/ExposeTools/ToolResult not dispatched — fell through to render buffer (#536)
- fix(changelog): accurate per-version deltas + bump-alpha anchors to last bump not last tag (#535)
- chore: DEV_LOG — entry for PR #534 (text input refocus + AI flush + env diagnostics)
- chore: bump alpha to 3.4.12, update changelog
- fix: text input refocus + flush outbound events same-frame + env probe diagnostics (#534)
- chore: DEV_LOG — record env-adoption failure honestly (still broken)
- chore: DEV_LOG — entry for PR #533 (zsh -i -l env probe)
- chore: DEV_LOG — entry for PR #532 (watchdog + drag-cursor fix)
- chore: bump alpha to 3.4.11, update changelog
- fix(shell): use -i -l for env probe so .zshrc-defined secrets load (#533)
- fix: tighten freeze watchdog + throttle macOS drag-cursor polling (#532)
- chore: DEV_LOG — add entry for PR #531
- chore: bump alpha to 3.4.10, update changelog
- fix: adopt login-shell env vars + TextInput layout widget (#531)
- chore: CLAUDE.md — add 'To test' line to Ship Cycle summary format
- chore: bump alpha to 3.4.9, update changelog
- chore: DEV_LOG — v3.7 complete (PR #526, closes #396, #398, #399, #516)
- feat(v3.7): app tool protocol — ExposeTools/ToolCall/ToolResult + host context injection (#526)
- chore: promote to beta — v3.4.8
- chore: CLAUDE.md — clarify post-merge workflow, standardize just bump-and-install, use GitHub issues over backlog
- chore: justfile bump-alpha fixes, changelog version labels, overlay opacity
- chore: bump alpha to 3.4.7, update changelog
- chore: bump alpha to 3.4.6, update changelog
- chore: DEV_LOG — changelog modal + bump-alpha (PR #524)
- feat(changelog): clickable version badge opens changelog modal + just bump-alpha (#524)
- chore: DEV_LOG — v3.8 partial batch (PRs #521-522, closes #388, #424)
- feat(error-visibility): boot timeout + render exception re-raise (#424, partial) (#522)
- feat(sdk/ui): ListItem and Row auto-centering components (#388) (#521)
- chore: DEV_LOG — v3.6 complete (PRs #518, #519, closes #508, #509)
- feat(#508): chat-poc — conversational chat via AiQuery/AiResponse (#519)
- chore(#509): delete stale docs/specs/, purge dangling references (#518)
- chore: DEV_LOG — WorkspaceRouter (#510, closes #380)
- refactor(#380): WorkspaceRouter — compile-enforced context switching invariant (#510)
- chore: DEV_LOG — v3.5 batch session (PRs #502-504, #380 deferred)
- docs: clarify alpha as starting branch for all changes
- fix(egui-term): empty clipboard guard, auto-scroll boundary, HiDPI column sync (#475, #492, #472) (#504)
- feat(#425): config migration on install (#503)
- fix(#429): delete orphaned src/plexi_iq/ (#502)
- chore(promote): use --force-with-lease instead of --force for alpha→beta
- chore(promote): force-push alpha→beta to handle diverged history
- chore: promote to beta — v3.4.3
- fix(install): rename binary in bundle for non-stable channels to match config detection
- feat(commit-graph): flat N-commit load, host scroll, badge overflow fix, PR badges (#500)
- feat(logging): heartbeat watchdog + workspace autosave on structural changes (#499)
- fix(ui): hide sidebar separator line — panel is not resizable (#483) (#495)
- fix(sidebar): remove hover sense from context label — eliminates I-beam cursor (#481) (#494)
- fix(ui): Escape dismisses shortcuts overlay (#484) (#496)
- fix(ui): align shortcuts overlay — use key_combo_list for HJKL rows, add min_col_width (#482) (#498)
- feat(toolbar): show app version label next to ? button (closes #485) (#497)
- chore: sync alpha version to 3.4.2
- refactor(promote): bump+changelog on alpha before push; aggregate all entries since last tag for GitHub release
- fix: awk newline-in-variable error in prepend_changelog — use temp file + getline
- fix: bash 3.2 compat in promote.sh — replace ${var,,} with explicit y/Y check
- feat: channel promotion pipeline (just promote)
- refactor(sidebar): zone-based row abstraction with single cursor authority

## [3.4.47] — 2026-05-03 15:34 ET

### Changes
- docs: DEV_LOG PR #580 + platform behavior validation lesson
- feat(macos): show version in menu bar for non-stable builds (#580)

## [3.4.46] — 2026-05-03 15:30 ET

### Changes
- docs: DEV_LOG PR #575 context naming modal when sidebar hidden
- docs: add contact section to README
- docs: fix DEV_LOG issue reference #580 → #582
- docs: clarify mit priority label — sits atop P1-P4, usually paired with P1

## [3.4.45] — 2026-05-03 15:21 ET

### Changes
- docs: add mit label to issue priority scheme in CLAUDE.md
- docs: DEV_LOG PR #581 drop-event breadcrumbs in zoomed overlay
- feat(logging): add drop-event breadcrumbs in zoomed overlay path (#581)

## [3.4.44] — 2026-05-03 14:58 ET

### Changes
- docs: add lesson about uncommitted bump on alpha

## [3.4.43] — 2026-05-03 14:40 ET

### Changes

## [3.4.42] — 2026-05-03 14:32 ET

### Changes
- chore: add ET timestamps to all changelog entries; emit time in bump-alpha (#574)

## [3.4.41] — 2026-05-03 14:26 ET

### Changes
- feat(widgets): dismissable_modal helper — escape + click-outside for overlays (#570)

## [3.4.40] — 2026-05-03 14:01 ET

### Changes

## [3.4.39] — 2026-05-03 13:59 ET

### Changes
- fix(terminal): explicitly push first char before iter_from in selectable_content (#569)

## [3.4.38] — 2026-05-03 13:47 ET

### Changes
- feat(pty): inject PLEXI_PANE_ID + PLEXI_SOCKET into every managed PTY environment (#565)

## [3.4.37] — 2026-05-03 13:37 ET

### Changes

## [3.4.36] — 2026-05-03 13:37 ET

### Changes
- feat: add issue template with Meta YAML convention for dependency tracking

## [3.4.35] — 2026-05-03 01:11 ET

### Changes
- feat(welcome): add Plexi logo + centered wordmark (#562)

## [3.4.34] — 2026-05-03 00:36 ET

### Changes

## [3.4.33] — 2026-05-03 00:35 ET

### Changes
- feat: add just pr-install and pr-clean for PR testing flow (#559)

## [3.4.32] — 2026-05-02 23:37 ET

### Changes
- dev_log: PR #556 sidebar hit-rect fix
- fix(sidebar): stabilise hit rects + clear renaming_window on reorder (#556)
- docs: add Failed PR reset protocol to CLAUDE.md
- Add project-level triage skill; unignore .claude/ directory
- docs: replace ship cycle with /ship skill reference, add testing label

## [3.4.31] — 2026-05-02 22:14 ET

### Changes
- dev_log: PR #551 tool registration + token diagnostics
- docs: add in-progress label to feature branch and ship cycle workflows

## [3.4.30] — 2026-05-02 22:13 ET

### Changes
- Add protocol scheme to OpenRouter HTTP-Referer header
- Improve bump message generation; simplify OpenRouter Referer header

## [3.4.29] — 2026-05-02 19:58 ET

### Changes
- Improve text input widget clamping and chat bubble sizing against pane bounds

## [3.4.28] — 2026-05-02 19:30 ET

### Changes
- Fetch complete OpenRouter generation metrics; fix text input widget clamping to pane bounds

## [3.4.27] — 2026-05-02 19:18 ET

### Changes

## [3.4.26] — 2026-05-02 19:18 ET

### Changes

## [3.4.25] — 2026-05-02 19:17 ET

### Changes
- chore: add DEV_LOG entry for PR #549 (chat UI improvements)

## [3.4.24] — 2026-05-02 19:17 ET

### Changes
- chore: update ship cycle to commit DEV_LOG before bump-and-install

## [3.4.23] — 2026-05-02 19:09 ET

### Changes
- fix: robust token parsing and deterministic tool dispatch
- feat(parallax): MVP editor app + SDK ctx.image() (#548)

## [3.4.22] — 2026-05-02 18:45 ET

### Changes
- fix: make @app.tool decorator cumulative + include usage in OpenRouter streams

## [3.4.21] — 2026-05-02 18:44 ET

### Changes
- fix: update OpenRouter HTTP-Referer to plexiapp.com
- chore: add DEV_LOG entry for PR #544 (chat UI, TextInput refocus fix)

## [3.4.20] — 2026-05-02 18:31 ET

### Changes
- feat(chat): polished bubble UI + TextInput refocus fix (#544)
- feat(input-inspector): add inputs page and per-category event filtering
- fix(logging): add info-level broker tool log, fix [log] section header in config
- chore: rename bundle to Plexi Alpha, default chat tier to low
- docs: update DEV_LOG with v3.7 context injection and TextInput fixes

## [3.4.19] — 2026-05-02 18:00 ET

### Changes
- feat(v3.7): context injection for all open panes, fix text cutoff and TextInput focus
- docs: consolidate project docs — remove ARCHITECTURE.md and ROADMAP.md, add GLOSSARY.md, update CLAUDE.md

## [3.4.18] — 2026-05-02 17:21 ET

### Changes
- fix(examples): log scroll events in input-inspector

## [3.4.17] — 2026-05-02 17:18 ET

### Changes
- feat(host): add TextRow draw command with host-measured text layout

## [3.4.16] — 2026-05-02 17:15 ET

### Changes
- DEV_LOG: log PR #540 text_row() layout primitive
- feat(sdk): add text_row() host-measured text layout primitive (#540)
- feat(examples): input-inspector POC for issue #331 (#529)
- chore: promote to beta — v3.4.14
- fix(promote.sh): auto-push unpushed commits instead of failing

## [3.4.15] — 2026-05-02 17:14 ET

### Changes
- feat(examples): input-inspector POC for issue #331 (#529)
- chore: promote to beta — v3.4.14
- fix(promote.sh): auto-push unpushed commits instead of failing

## [3.4.14] — 2026-05-02 16:24 ET

### Changes
- fix(promote.sh): auto-push unpushed commits instead of failing
- chore: bump alpha to 3.4.13, update changelog
- fix(justfile): bump-alpha SIGPIPE with pipefail — use git log -1 not | head -1 (#537)
- fix(routing): AiQuery/ExposeTools/ToolResult not dispatched — fell through to render buffer (#536)
- fix(changelog): accurate per-version deltas + bump-alpha anchors to last bump not last tag (#535)
- chore: DEV_LOG — entry for PR #534 (text input refocus + AI flush + env diagnostics)
- chore: bump alpha to 3.4.12, update changelog
- fix: text input refocus + flush outbound events same-frame + env probe diagnostics (#534)
- chore: DEV_LOG — record env-adoption failure honestly (still broken)
- chore: DEV_LOG — entry for PR #533 (zsh -i -l env probe)
- chore: DEV_LOG — entry for PR #532 (watchdog + drag-cursor fix)
- chore: bump alpha to 3.4.11, update changelog
- fix(shell): use -i -l for env probe so .zshrc-defined secrets load (#533)
- fix: tighten freeze watchdog + throttle macOS drag-cursor polling (#532)
- chore: DEV_LOG — add entry for PR #531
- chore: bump alpha to 3.4.10, update changelog
- fix: adopt login-shell env vars + TextInput layout widget (#531)
- chore: CLAUDE.md — add 'To test' line to Ship Cycle summary format
- chore: bump alpha to 3.4.9, update changelog
- chore: DEV_LOG — v3.7 complete (PR #526, closes #396, #398, #399, #516)
- feat(v3.7): app tool protocol — ExposeTools/ToolCall/ToolResult + host context injection (#526)
- chore: promote to beta — v3.4.8
- chore: CLAUDE.md — clarify post-merge workflow, standardize just bump-and-install, use GitHub issues over backlog
- chore: justfile bump-alpha fixes, changelog version labels, overlay opacity
- chore: bump alpha to 3.4.7, update changelog
- chore: bump alpha to 3.4.6, update changelog
- chore: DEV_LOG — changelog modal + bump-alpha (PR #524)
- feat(changelog): clickable version badge opens changelog modal + just bump-alpha (#524)
- chore: DEV_LOG — v3.8 partial batch (PRs #521-522, closes #388, #424)
- feat(error-visibility): boot timeout + render exception re-raise (#424, partial) (#522)
- feat(sdk/ui): ListItem and Row auto-centering components (#388) (#521)
- chore: DEV_LOG — v3.6 complete (PRs #518, #519, closes #508, #509)
- feat(#508): chat-poc — conversational chat via AiQuery/AiResponse (#519)
- chore(#509): delete stale docs/specs/, purge dangling references (#518)
- chore: DEV_LOG — WorkspaceRouter (#510, closes #380)
- refactor(#380): WorkspaceRouter — compile-enforced context switching invariant (#510)
- chore: DEV_LOG — v3.5 batch session (PRs #502-504, #380 deferred)
- docs: clarify alpha as starting branch for all changes
- fix(egui-term): empty clipboard guard, auto-scroll boundary, HiDPI column sync (#475, #492, #472) (#504)
- feat(#425): config migration on install (#503)
- fix(#429): delete orphaned src/plexi_iq/ (#502)
- chore(promote): use --force-with-lease instead of --force for alpha→beta
- chore(promote): force-push alpha→beta to handle diverged history
- chore: promote to beta — v3.4.3
- fix(install): rename binary in bundle for non-stable channels to match config detection
- feat(commit-graph): flat N-commit load, host scroll, badge overflow fix, PR badges (#500)
- feat(logging): heartbeat watchdog + workspace autosave on structural changes (#499)
- fix(ui): hide sidebar separator line — panel is not resizable (#483) (#495)
- fix(sidebar): remove hover sense from context label — eliminates I-beam cursor (#481) (#494)
- fix(ui): Escape dismisses shortcuts overlay (#484) (#496)
- fix(ui): align shortcuts overlay — use key_combo_list for HJKL rows, add min_col_width (#482) (#498)
- feat(toolbar): show app version label next to ? button (closes #485) (#497)
- chore: sync alpha version to 3.4.2
- refactor(promote): bump+changelog on alpha before push; aggregate all entries since last tag for GitHub release
- fix: awk newline-in-variable error in prepend_changelog — use temp file + getline
- fix: bash 3.2 compat in promote.sh — replace ${var,,} with explicit y/Y check
- feat: channel promotion pipeline (just promote)
- refactor(sidebar): zone-based row abstraction with single cursor authority

## [3.4.13] — 2026-05-02 06:17 ET

### Changes
- fix(justfile): bump-alpha SIGPIPE with pipefail — use git log -1 not | head -1 (#537)
- fix(routing): AiQuery/ExposeTools/ToolResult not dispatched — fell through to render buffer (#536)
- fix(changelog): accurate per-version deltas + bump-alpha anchors to last bump not last tag (#535)

## [3.4.12] — 2026-05-02 05:33 ET

### Changes
- fix: text input refocus + flush outbound events same-frame + env probe diagnostics (#534)

## [3.4.11] — 2026-05-02 04:46 ET

### Changes
- fix(shell): use -i -l for env probe so .zshrc-defined secrets load (#533)
- fix: tighten freeze watchdog + throttle macOS drag-cursor polling (#532)

## [3.4.10] — 2026-05-02 04:22 ET

### Changes
- fix: adopt login-shell env vars + TextInput layout widget (#531)
- chore: CLAUDE.md — add 'To test' line to Ship Cycle summary format

## [3.4.9] — 2026-05-02 04:01 ET

### Changes
- feat(v3.7): app tool protocol — ExposeTools/ToolCall/ToolResult + host context injection (#526)
- chore: promote to beta — v3.4.8
- chore: CLAUDE.md — clarify post-merge workflow, standardize just bump-and-install, use GitHub issues over backlog
- chore: justfile bump-alpha fixes, changelog version labels, overlay opacity

## [3.4.8] — 2026-05-02 03:40 ET

### Changes
- chore: CLAUDE.md — clarify post-merge workflow, standardize just bump-and-install, use GitHub issues over backlog
- chore: justfile bump-alpha fixes, changelog version labels, overlay opacity

## [3.4.6] — 2026-05-02 03:34 ET

### Changes
- feat(changelog): clickable version badge opens changelog modal + just bump-alpha (#524)

## [3.4.3] — 2026-05-02 00:08 ET

### Changes
- fix(install): rename binary in bundle for non-stable channels to match config detection
- feat(commit-graph): flat N-commit load, host scroll, badge overflow fix, PR badges (#500)
- feat(logging): heartbeat watchdog + workspace autosave on structural changes (#499)
- fix(ui): hide sidebar separator line — panel is not resizable (#483) (#495)
- fix(sidebar): remove hover sense from context label — eliminates I-beam cursor (#481) (#494)
- fix(ui): Escape dismisses shortcuts overlay (#484) (#496)
- fix(ui): align shortcuts overlay — use key_combo_list for HJKL rows, add min_col_width (#482) (#498)
- feat(toolbar): show app version label next to ? button (closes #485) (#497)
- chore: sync alpha version to 3.4.2
- refactor(promote): bump+changelog on alpha before push; aggregate all entries since last tag for GitHub release
- fix: awk newline-in-variable error in prepend_changelog — use temp file + getline
- fix: bash 3.2 compat in promote.sh — replace ${var,,} with explicit y/Y check
- feat: channel promotion pipeline (just promote)
- refactor(sidebar): zone-based row abstraction with single cursor authority

## [3.4.1] — 2026-05-01 16:31 ET

### Changes
- **Command palette overhaul** — context/pane model rewrite; named pane entries with direct focus jump; strip stale auto window names on load
- **Remove pulse beta feature** — `pulse` config flag and breathing border effect removed
- **Fix `Cmd+Shift+,` reload shortcut** — wired through macOS menu NSEventModifierMask; was unreliable via egui key handling
- **Fix `theme_preset` TOML ordering** — was silently ignored when placed after a section header in config template
- **Remove built-in text editor** — `TextEditorApp` deleted; `Cmd+,` now opens config in system editor
- **Compile gate on `just bump`** — `cargo build --release` runs before tagging so broken builds can't reach a release
- **Square key chips** — shortcut chips resize to square for single-char keys
- **Shortcuts overlay** — two-column layout, HJKL navigation blocks, wider overlay
- **Minimap** — page numbers switched to 0-based

## [3.4.0] — 2026-05-01 16:03 ET

### Features
- **Bundled Python 3.12** — self-contained runtime via python-build-standalone; no system Python dependency
- **Navigation stack** — `PushNav` / `PopNav` / `NavBack` protocol for multi-screen app flows
- **Async SDK** — Python SDK event loop is fully async; eliminates blocking-in-event-loop deadlocks
- **Host-managed `ScrollRegion`** — primitive for smooth scrollable app content without manual offset tracking
- **Mouse events in apps** — `PlexiEvent::Mouse*` now fires correctly inside app panes
- **`TextInput` primitive** — host-owned single-line entry with auto-focus and Shift+Enter multiline
- **Sidebar context rename** — double-click to rename; auto-rename on new context creation
- **Parallax editor app** — GUI wrapper for the Parallax video editor pipeline
- **Agent Workspace** — modal UI for spawning Claude Code agents with repo context
- **App registry** — directory-scoped app + agent discovery
- **Workspace-scoped secret routing** — secrets namespace to the active `.plexi/` workspace
- **Workspace config merge** — per-project `.plexi/config.toml` merged with global config
- **App package manager** — `install` / `uninstall` / `update` / `list` with bundled core pack
- **App lifecycle pill** — observable running/stopped indicator per app
- **OpenRouter AI backend** — configurable model tiers, real cost tracking
- **CoreMIDI I/O** — typed pipe for MIDI in/out on macOS
- **CoreAudio capture** — cpal-backed device enumeration and PCM capture
- **AVFoundation video decoder** — native macOS video playback backing
- **Hot reload** — live app reload on source change during development
- **Agent roster + inter-agent pipes** — directed communication between running agents
- **Cmd+N split-mirror + lateral focus** — `Shift+Cmd+H/J/K/L` pane navigation

### Infrastructure
- **Smart `just install`** — reads `.channel` file and dispatches to the right channel automatically
- **Canonical source identity** — `Cargo.toml` + `src/main.rs` use generic names; channel applied at build time via `sed` + restore trap; eliminates merge conflicts between alpha/beta/main
- **`.channel` + `merge=ours`** — per-branch identity file protected from merge overwrites

### Fixes
- Python SDK path corrected in `.app` bundle
- Quit freeze resolved — subscription busy-loop + child reap moved off render thread
- Empty context welcome screen on all windows
- Sidebar drag-drop reordering with visual drop indicator
- SDK clean shutdown + HiDPI hit-test alignment
- Terminal scrollback clearing on alt-screen entry

## [3.0.0-beta.5] — 2026-04-25 19:41 ET

### Host / Notifications

- **Background app cross-context tick fix** — Global notification apps running in the background were not receiving tick events when the active pane changed context. Fix ensures the tick is dispatched correctly regardless of which context is active.

## [3.0.0-beta.4] — 2026-04-24 04:33 ET

### Apps

- **Broken apps fixed** — `notification-tester`, `screen-time`, and `stand-up-reminder` all crashed on launch due to syntax errors or missing `Component` inheritance after the AppBar migration. All three now boot cleanly.
- **Custom component fix** — `_CountdownRing` (stand-up-reminder), `_Body` (quick-note, todo, wikipedia) now inherit `Component` so `_render_clipped` is available. Same class of bug fixed in `ui-playground`.
- **SDK bug fix** — `Scrollable.render` was orphaned outside the class body after a bad indent; moved back in.
- **Deleted lava-lamp, lava-opus, audio-recorder** — removed from examples and installed app dirs.

### SDK

- **`ensure_visible(scroll_offset, viewport_h, top, bottom, margin=0) → float`** free function in `plexi_sdk.ui`. Canonical one-line solve for selection-follows-scroll in any scrollable list. `Scrollable.ensure_visible()` wraps it as a method. Commit Graph j/k/g/G nav handlers migrated to use it.
- **`_render_clipped` on `Component`** — base class now clips every child to its allocated rect via PushClip/PopClip before calling `render`. Custom components must inherit `Component` or they will crash when placed in a `Column`.

### Host / Notification modal

- Keyboard hint row is now centered (was left-aligned due to `horizontal_wrapped` inside `vertical_centered`).
- Acknowledge button tightened: 220px → 180px, spacing above reduced from 24px → 12px.

### Host / Rendering

- `render_draw_commands` takes `pane_rect` explicitly — single source of geometry, no more `ui.min_rect()` surprise.
- Clip stack (PushClip/PopClip) intersects with current stack top so nested clips only ever tighten.

## [3.0.0-beta.3] — 2026-04-23 23:30 ET

### Notifications — major rework

- **Pinned-by-id queue.** The currently-displayed notification is pinned by id and never displaced by incoming ones. New notifications arriving bump the count live on screen but don't yank your view to something else.
- **Priority-sorted dismiss.** Four tiers exported from `plexi_sdk`: `PRIORITY_LOW=0`, `PRIORITY_NORMAL=50`, `PRIORITY_HIGH=100`, `PRIORITY_CRITICAL=200`. After dismiss, the next front-most is chosen from whatever's in the queue *right now* — highest priority wins, arrival breaks ties.
- **Interrupt threshold (`[notifications] interrupt_threshold`).** Defaults to `100`: NORMAL and LOW queue silently (badge ticks, Cmd+Shift+A reveals), HIGH and CRITICAL auto-open the modal. Set to `0` for the old everything-interrupts behaviour; set to `201` to match `focus_mode`.
- **Esc defers, Enter acknowledges.** Previously both destroyed the notification. Now Esc closes the modal but keeps the notification in the queue (Cmd+Shift+A brings it back). Enter / option-select / input-submit still dispatches `NotifyAction` and removes from queue. Required notifications still reject Esc.
- **Cross-context scope.** New manifest field `[app] default_notification_scope = "context" | "global"`. `global` notifications (e.g. stand-up reminders) are visible regardless of active context; `context` notifications (e.g. "note saved") stay local. Apps never see scope — users control it by editing the manifest.
- **Cross-context drain.** All app panes in all contexts drain commands every frame. Previously background-context apps silently buffered notifications until you switched contexts — fixed.
- **Per-context sidebar badges.** Inactive contexts show their own context-scoped notification count.

### Apps

- **Commit Graph v2** — subway-style layout with viewport-scoped lanes (no more 18-lane noise from silent refs), hard-capped at 5 lanes with an `other` collapse bucket, fixed right-hand label column with hard truncation, hollow-diamond glyph for merge commits, parent-side colouring for non-mainline merge edges, Enter toggles tooltip, click-away clears.
- **PlexiEvent::Click forwarding** — process apps now receive pane clicks as structured events. `balls` demo gained click-to-remove.
- Six example apps migrated to SDK v2 declarative UI components (Column / Card / Header / KeyRow / FooterKeys / Section / Spacer / Footer).

### SDK + UI

- **`key_chip` primitive** (`src/widgets.rs`) — single rounded keycap pill with subtle border + monospace label. `key_combo` / `key_combo_list` wrappers built on top. Migrated every host-side shortcut hint: `?` help overlay, notification modal hint, confirm-close footer, command palette, run palette, rename-pane.
- **Python SDK `KeyRow`** — accepts `str | list[str]`, renders chips matching the host. Picked up by the Commit Graph help overlay and example-app footers.
- **`badge()` free function** + **`FooterKeys` component** in `plexi_sdk.ui`. Ends the "every app draws its own pill shape slightly wrong" class of bug. Commit Graph, todo, stand-up-reminder, wikipedia, quick-note, screen-time all migrated.
- **SDK docs pass** — new `NOTIFICATIONS` block and `MANIFEST REFERENCE` block in the module docstring covering kinds, priority guidance, queue model, scope model, `NotifyAction` round-trip, and every `[app]` manifest field.

### Host

- **GUI-bundle PATH fix** — launching Plexi from `/Applications` inherits only `/usr/bin:/bin:/usr/sbin:/sbin`. Apps shelling out to `gh` / `rg` / `fd` couldn't find them. New `shell::install_login_shell_path` resolves the user's login-shell PATH at startup and adopts it process-wide. Fallback prepends `/opt/homebrew/{bin,sbin}:/usr/local/{bin,sbin}` on macOS if the probe fails.
- **`tiling.rs` decomposed** — split into `render/{terminal,app,agent}_pane.rs`; `pane_ops.rs` split into `create / layout / workspace` submodules. Pure refactor.
- **Debug-level log targets expanded** — `plexi_alpha` and `app::<id>` targets now follow the configured log level. Previously only `plexi` / `plexi_v3` did; per-app debug/info lines were silently dropped.

### Fixes

- `git log --format` must use `%x00` / `%x01` escapes, not literal NUL bytes in argv — Commit Graph's "No commits this week" bug.
- Shebang must be line 1, not after `from __future__`.
- Spinner derives frame index from wall-clock monotonic, not per-render count.
- URL hyperlink detection across client-wrapped terminal rows.
- Drag-and-drop files onto a fullscreen pane lands in the zoomed terminal instead of silently writing to a background tile.
- Header top padding tightened (16px → 8px).
- Grey square in every pane's top-left corner (collapsing `egui::Frame` wrappers replaced with direct `ui.painter()` calls).
- Modal focus leak on Cmd+W confirm (and palette / rename) — migrated to FocusLayer with `consume_key`.
- Command palette no longer collapses to one row with a single pane.

### Justfile

- **`just clear-apps <channel>`** — explicit mirror-install helper. `cp -R` in `install-*` is sync-not-mirror; apps deleted from `examples/` don't disappear from the install dir on upgrade. Run `just clear-apps alpha && just install-alpha` for a clean slate.

## [3.0.0-beta.2] — 2026-04-23 07:51 ET

### Added
- **Commit Graph app (github-tree, replaces file browser)** — subway-style git history viewer for the pane's repo. Vertical time, horizontal branch lanes, weekly viewport with `[` / `]` navigation, click-to-select with full message + diff-stat tooltip. No `gh` dependency — pure local git. Ships as `github-tree` (same launch slot as before).
- **PlexiEvent::Click forwarding** — process apps now receive pane clicks as structured events. `balls` demo gained click-to-remove so this is visible end-to-end.
- **SDK v2 declarative UI components** — `Column`, `Header`, `Card`, `KeyRow`, `Section`, `Spacer`, `Footer` plus design tokens (`SPACE_*`, `TEXT_*`). Six example apps migrated.
- **Multi-kind notification modal** — work-area surface for `ctx.notify`, `ctx.notify_choice`, `ctx.notify_input`, with keyboard-first navigation.

### Fixed
- **Host PATH under GUI-bundle launch** — launching from `/Applications` inherited only `/usr/bin:/bin:/usr/sbin:/sbin`, so apps shelling out to `gh` / `rg` / `fd` couldn't find them. Now resolves the user's login-shell PATH once at startup and adopts it process-wide.
- **Grey square in every pane's top-left** — collapsing `egui::Frame` wrappers in `process_app` and `tiling` have been dropped; pane backgrounds are now painted directly over `available_rect_before_wrap`.
- **Modal keyboard focus leak (Cmd+W confirm + palette + rename overlays)** — overlays now consume keys via the `FocusLayer` pipeline instead of read-only `ui.input(key_pressed)`. Hitting Enter on the close confirm no longer triggers the selected-item action in the pane behind it.
- **Command palette collapsing to one row with a single pane** — `ScrollArea` now pairs `max_height` with `min_scrolled_height`, so the viewport stays full-size regardless of content count.
- **Terminal URL hyperlink detection across wrapped rows** — client-wrapped URLs (e.g. Claude Code output) are now detected as single links spanning both rows. Cmd+click opens the full URL. Copy-path unchanged — that's tracked separately as v3.1.
- **Drag-drop on a fullscreen pane** — files dragged onto a zoomed pane now land in the zoomed terminal instead of silently writing to a background tile.
- **Screen Time rework** — 15-min buckets, clock hand, day-boundary bleed fix, SDK v2 chrome.

### Changed
- **Tiling decomposition** — `tiling.rs` split into `render/{terminal,app,agent}_pane.rs`; `pane_ops.rs` broken up into `create / layout / workspace` submodules. Pure refactor, zero behavior change.
- **Header top padding** — `Column.padding_top` default dropped from 16px to 8px so top-of-pane headers feel anchored instead of dropped.

## [1.1.2] — 2026-04-10 17:19 ET

### Fixed
- **Cloud folder crash** — file browser no longer freezes when opening Google Drive, iCloud, or other FUSE-backed cloud folders. Eliminated per-entry `stat` syscalls in favor of cached directory entry types.
- **PTY escape query hangs** — programs like fzf that query cursor position or text area size no longer hang waiting for a response.

### Improved
- **CWD tracking performance** — cached `lsof` lookups with 300ms TTL instead of calling every frame.

## [1.1.1] — 2026-04-10 08:45 ET

### Added
- **Theme presets** — set `theme_preset = "dracula"` (or `catppuccin-mocha`, `tokyo-night`, `gruvbox-dark`, `nord`, `solarized-dark`) in `config.toml` to apply a full UI + terminal color scheme. Individual `[theme]` overrides layer on top.
- **CRT & pulse effects** — opt-in via `[beta]` section in `config.toml`. `crt = true` adds green phosphor tint + scanlines. `pulse = true` animates the focused pane border.
- **`just install-alpha` / `just install-beta`** — build and install variant app bundles (`Plexi Alpha.app`, `Plexi Beta.app`) with fully isolated config directories (`~/.plexi-alpha`, `~/.plexi-beta`). Deprecates `just install-apps`.

## [1.1.0] — 2026-04-10 08:45 ET

### Added
- Cmd+Comma opens config in embedded text editor.
- Inline text editing in file browser sidebar.
- Standalone text editor app.

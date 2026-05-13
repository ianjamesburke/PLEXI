# Changelog

Newest releases appear first.
## [3.6.65] — 2026-05-13

### Changes
- refactor(host): extract test modules from app/mod.rs and process_app/mod.rs (#1261) (#1277)
- docs: consolidate issue label convention into create-issue skill
- fix(triage): add explicit era label enumeration step to prevent v3.1+ default (#1268)
- fix(minimap): number cells by spatial position, not array index (#1275)
- feat(sdk): runtime isinstance check in Column/Card/Scrollable (#1202) (#1269)
- feat: cancel active StreamProcess children on ProcessApp drop (#675) (#1270)
- docs: add Railway build configuration gotcha and ignore local config dirs
- fix(docs): remove nonexistent `plexi note` CLI section from Quick Note page
- fix(terminal): remove 8px inner padding that caused background color fringe (#1265)
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.64] — 2026-05-13

### Changes
- docs: consolidate issue label convention into create-issue skill
- fix(triage): add explicit era label enumeration step to prevent v3.1+ default (#1268)
- fix(minimap): number cells by spatial position, not array index (#1275)
- feat(sdk): runtime isinstance check in Column/Card/Scrollable (#1202) (#1269)
- feat: cancel active StreamProcess children on ProcessApp drop (#675) (#1270)
- docs: add Railway build configuration gotcha and ignore local config dirs
- fix(docs): remove nonexistent `plexi note` CLI section from Quick Note page
- fix(terminal): remove 8px inner padding that caused background color fringe (#1265)
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.63] — 2026-05-13

### Changes
- fix(minimap): number cells by spatial position, not array index (#1275)
- feat(sdk): runtime isinstance check in Column/Card/Scrollable (#1202) (#1269)
- feat: cancel active StreamProcess children on ProcessApp drop (#675) (#1270)
- docs: add Railway build configuration gotcha and ignore local config dirs
- fix(docs): remove nonexistent `plexi note` CLI section from Quick Note page
- fix(terminal): remove 8px inner padding that caused background color fringe (#1265)
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.62] — 2026-05-13

### Changes
- feat(sdk): runtime isinstance check in Column/Card/Scrollable (#1202) (#1269)
- feat: cancel active StreamProcess children on ProcessApp drop (#675) (#1270)
- docs: add Railway build configuration gotcha and ignore local config dirs
- fix(docs): remove nonexistent `plexi note` CLI section from Quick Note page
- fix(terminal): remove 8px inner padding that caused background color fringe (#1265)
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.61] — 2026-05-13

### Changes
- feat: cancel active StreamProcess children on ProcessApp drop (#675) (#1270)
- docs: add Railway build configuration gotcha and ignore local config dirs
- fix(docs): remove nonexistent `plexi note` CLI section from Quick Note page
- fix(terminal): remove 8px inner padding that caused background color fringe (#1265)
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.60] — 2026-05-13

### Changes
- fix(terminal): remove 8px inner padding that caused background color fringe (#1265)
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.59] — 2026-05-13

### Changes
- feat(cli): add `plexi pane self` and suppress CLI stderr log noise (#1266)
- fix(website): move railway.json back to website/ where Railway expects it
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.58] — 2026-05-13

### Changes
- feat(welcome): add ⌘P to shortcuts, remove right-click tip (#698) (#1267)
- fix(website): move railway.json to repo root for new build context
## [3.6.57] — 2026-05-13

### Changes
- docs: add Plexi Stats app design spec (#1262)
- feat(config): validate config.toml and surface errors (#1116) (#1259)
- docs: add DEV_LOG entry for SDK doc generator (#1258)
- feat(sdk): self-documenting SDK — typed surface + auto-generated docs website (#1248) (#1258)
- fix(ship): ship-cycle hygiene — delete stale docs, fix smoke test, remove agent waste (#1250, #1251, #1252, #1253) (#1257)
- feat(config): hot-reload config.toml on save without restart (#1115) (#1249)
- feat(sidebar): set/clear context root from right-click menu (#1133) (#1247)
- fix(host): bind ArrowRight/ArrowLeft in quick-note destination pickers (#1153) (#1246)
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.56] — 2026-05-13

### Changes
- feat(config): validate config.toml and surface errors (#1116) (#1259)
- docs: add DEV_LOG entry for SDK doc generator (#1258)
- feat(sdk): self-documenting SDK — typed surface + auto-generated docs website (#1248) (#1258)
- fix(ship): ship-cycle hygiene — delete stale docs, fix smoke test, remove agent waste (#1250, #1251, #1252, #1253) (#1257)
- feat(config): hot-reload config.toml on save without restart (#1115) (#1249)
- feat(sidebar): set/clear context root from right-click menu (#1133) (#1247)
- fix(host): bind ArrowRight/ArrowLeft in quick-note destination pickers (#1153) (#1246)
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.55] — 2026-05-13

### Changes
- fix(ship): ship-cycle hygiene — delete stale docs, fix smoke test, remove agent waste (#1250, #1251, #1252, #1253) (#1257)
- feat(config): hot-reload config.toml on save without restart (#1115) (#1249)
- feat(sidebar): set/clear context root from right-click menu (#1133) (#1247)
- fix(host): bind ArrowRight/ArrowLeft in quick-note destination pickers (#1153) (#1246)
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.54] — 2026-05-13

### Changes
- feat(sidebar): set/clear context root from right-click menu (#1133) (#1247)
- fix(host): bind ArrowRight/ArrowLeft in quick-note destination pickers (#1153) (#1246)
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.53] — 2026-05-13

### Changes
- fix(host): bind ArrowRight/ArrowLeft in quick-note destination pickers (#1153) (#1246)
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.52] — 2026-05-13

### Changes
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.51] — 2026-05-13

### Changes
- fix(host): paste leaks through overlays to terminal behind modal (#1236) (#1242)
- fix(scratchpad): leading space on open, Cmd+X broken, save path fallback (#1231) (#1241)
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.50] — 2026-05-13

### Changes
- fix(cli): sync shell completion scripts with current CLI surface (#1240) (#1243)
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.49] — 2026-05-13

### Changes
- Revert "feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)" (#1237)
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.48] — 2026-05-13

### Changes
- feat(cli): plexi pane capture — read PTY scrollback from any pane (#978) (#1234)
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.47] — 2026-05-13

### Changes
- feat(apps): minimal native text editor — TextArea + TextBuffer, plexi open editor [file] (#1099) (#1232)
- feat(theme): add bg_sidebar_hover surface token for sidebar hover contrast (#1233)
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.46] — 2026-05-13

### Changes
- feat(host): double-spacebar scratchpad — instant capture-to-file text editor (#1213) (#1230)
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.45] — 2026-05-13

### Changes
- feat(apps): gh-projects read-only GitHub Projects Kanban board (#1225) (#1226)
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.44] — 2026-05-13

### Changes
- feat(terminal): add --no-focus flag to prevent new pane from stealing focus (#1192) (#1224)
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.43] — 2026-05-13

### Changes
- chore: update ship-issue skill
- feat(quick-note): recursive menus, bare-command leaves, and dynamic children (#1212) (#1222)
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.42] — 2026-05-13

### Changes
- feat(pane-ops): Cmd+Ctrl+J/K sends pane to first/last column (#1079) (#1221)
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.41] — 2026-05-13

### Changes
- feat(event-log): FocusChanged event — pane attention trace (#1150) (#1220)
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.40] — 2026-05-13

### Changes
- fix(ship-issue): verify stash-pop files are PR-related before auto-committing (#1006) (#1218)
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.39] — 2026-05-13

### Changes
- chore: restore session edits carried through merge
- feat(docs): generate CLI reference from clap via gen_cli_docs tool (#1219)
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.38] — 2026-05-13

### Changes
- feat(quick-note): add stay_alive option for pane destinations (#1216)
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.37] — 2026-05-12

### Changes
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.37] — 2026-05-12

### Changes
- fix(render): downgrade render_layout_node per-frame logs to debug (#1215)
- fix(quick-note): same-frame draw on H/Esc exit to eliminate scrim flicker (#1214)
## [3.6.36] — 2026-05-12

### Changes
- feat(terminal): respect OSC 0/1/2 pane title sequences (#795) (#1211)
- feat(app-init): use layout API in generated template (#1207) (#1209)
- ux(inputs): extend text input polish to command palette, sidebar rename, and process app fields (#1139) (#1210)
- feat(render): implement PGAP image rendering — eliminate protocol no-op (#1144) (#1206)
- fix: gate submit_text_input under #[cfg(test)] — not called in production after render refactor
- refactor(render): introduce RenderSession — consolidate widget passes (#1145) (#1205)
- chore: add remote branch ownership check to ship skill and justfile ship recipe
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.35] — 2026-05-12

### Changes
- feat(app-init): use layout API in generated template (#1207) (#1209)
- ux(inputs): extend text input polish to command palette, sidebar rename, and process app fields (#1139) (#1210)
- feat(render): implement PGAP image rendering — eliminate protocol no-op (#1144) (#1206)
- fix: gate submit_text_input under #[cfg(test)] — not called in production after render refactor
- refactor(render): introduce RenderSession — consolidate widget passes (#1145) (#1205)
- chore: add remote branch ownership check to ship skill and justfile ship recipe
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.34] — 2026-05-12

### Changes
- ux(inputs): extend text input polish to command palette, sidebar rename, and process app fields (#1139) (#1210)
- feat(render): implement PGAP image rendering — eliminate protocol no-op (#1144) (#1206)
- fix: gate submit_text_input under #[cfg(test)] — not called in production after render refactor
- refactor(render): introduce RenderSession — consolidate widget passes (#1145) (#1205)
- chore: add remote branch ownership check to ship skill and justfile ship recipe
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.33] — 2026-05-12

### Changes
- feat(render): implement PGAP image rendering — eliminate protocol no-op (#1144) (#1206)
- fix: gate submit_text_input under #[cfg(test)] — not called in production after render refactor
- refactor(render): introduce RenderSession — consolidate widget passes (#1145) (#1205)
- chore: add remote branch ownership check to ship skill and justfile ship recipe
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.32] — 2026-05-12

### Changes
- fix: gate submit_text_input under #[cfg(test)] — not called in production after render refactor
- refactor(render): introduce RenderSession — consolidate widget passes (#1145) (#1205)
- chore: add remote branch ownership check to ship skill and justfile ship recipe
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.31] — 2026-05-12

### Changes
- refactor(render): introduce RenderSession — consolidate widget passes (#1145) (#1205)
- chore: add remote branch ownership check to ship skill and justfile ship recipe
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.30] — 2026-05-12

### Changes
- fix(app-registry): home dir stop guard must fire before .plexi check (#1064) (#1199)
- feat(terminal): add --cwd flag to plexi terminal for explicit working directory (#1200)
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.29] — 2026-05-12

### Changes
- security(net): harden allowed_hosts matching, redirects, and URL parsing (#1195)
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.28] — 2026-05-12

### Changes
- fix(notifications): consume modal keys so they don't bleed into panes (#1197) (#1198)
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.27] — 2026-05-12

### Changes
- security(typed-pipes): sanitize pipe IDs and harden socket allocation (#1180) (#1193)
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.26] — 2026-05-12

### Changes
- security(permissions): canonicalize workspace roots before keying permission decisions (#1176) (#1194)
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.25] — 2026-05-12

### Changes
- fix(terminal): pass initial_cmd to new_window and tab layouts (#1184) (#1191)
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.24] — 2026-05-12

### Changes
- security(mcp): harden per-app MCP server binding and caller authentication (#1190)
- feat(skills): add /daily-check skill and docs-freshness tracker (#1173)
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.23] — 2026-05-12

### Changes
- security(ai-tools): enforce workspace-scoped tool authorization (#1182) (#1188)
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.22] — 2026-05-12

### Changes
- security(app-registry): harden app shadowing and linked app provenance (#1178) (#1186)
- security(terminal-bindings): enforce linked-terminal ownership and cwd boundaries (#1175) (#1189)
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.21] — 2026-05-12

### Changes
- security(shell): inventory all sh -c paths, add StreamProcess denial test (#1177) (#1187)
- security(app-env): stop injecting user-global secrets into app envs (#1167) (#1185)
- fix(ship-issue): use native gh-issue-ext blocking check in Phase 6 unblock step
- chore(skills): add GitHub project board status updates to ship cycle
## [3.6.20] — 2026-05-12

### Changes
- feat(website): adopt Tailwind v4 + shadcn tokens + typography (#1172) (#1174)
- fix(install): avoid mutating Cargo manifest for channel bundles
- chore(justfile): add web recipe for website dev server
- feat(docs): scaffold docs system with content collection, layout, and seed pages
- chore(website): remove MIT license and built-in-public from footer
- chore(scripts): report orphaned worktrees in pr-clean-merged (#1166)
## [3.6.19] — 2026-05-12

### Changes
## [3.6.18] — 2026-05-12

### Changes
- docs(readme): fix swap-pane shortcut — Cmd+Ctrl+K sends to front, not H
- docs(readme): add Cmd+Ctrl+H/J pane send-to-front/back shortcuts
- chore: remove plexi-orchestrator skill
- feat(cli): add -e short flag for --ephemeral on terminal command (#1162)
- fix(overlays): cli setup modal captures keyboard input via FocusLayer (#1142) (#1147)
- feat(swap-pane): Cmd+Ctrl+J at bottom boundary pops pane to new window (#1161)
- docs(readme): overhaul — add Quick Note, Notifications, App init, PGAP, Secrets, CLI reference sections; keep under-construction header with pre-v1 note (#1154) (#1159)
- feat(terminal): new_window layout — parallel dispatch spawns windows not splits (#1158)
- fix(close): advance focus to next sibling on pane/page close (#1152) (#1155)
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.17] — 2026-05-12

### Changes
- docs(readme): fix swap-pane shortcut — Cmd+Ctrl+K sends to front, not H
- docs(readme): add Cmd+Ctrl+H/J pane send-to-front/back shortcuts
- chore: remove plexi-orchestrator skill
- feat(swap-pane): Cmd+Ctrl+J at bottom boundary pops pane to new window (#1161)
- docs(readme): overhaul — add Quick Note, Notifications, App init, PGAP, Secrets, CLI reference sections; keep under-construction header with pre-v1 note (#1154) (#1159)
- feat(terminal): new_window layout — parallel dispatch spawns windows not splits (#1158)
- fix(close): advance focus to next sibling on pane/page close (#1152) (#1155)
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.16] — 2026-05-12

### Changes
- docs(readme): add Cmd+Ctrl+H/J pane send-to-front/back shortcuts
- chore: remove plexi-orchestrator skill
- feat(swap-pane): Cmd+Ctrl+J at bottom boundary pops pane to new window (#1161)
- docs(readme): overhaul — add Quick Note, Notifications, App init, PGAP, Secrets, CLI reference sections; keep under-construction header with pre-v1 note (#1154) (#1159)
- feat(terminal): new_window layout — parallel dispatch spawns windows not splits (#1158)
- fix(close): advance focus to next sibling on pane/page close (#1152) (#1155)
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.15] — 2026-05-12

### Changes
- feat(swap-pane): Cmd+Ctrl+J at bottom boundary pops pane to new window (#1161)
- docs(readme): overhaul — add Quick Note, Notifications, App init, PGAP, Secrets, CLI reference sections; keep under-construction header with pre-v1 note (#1154) (#1159)
- feat(terminal): new_window layout — parallel dispatch spawns windows not splits (#1158)
- fix(close): advance focus to next sibling on pane/page close (#1152) (#1155)
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.14] — 2026-05-12

### Changes
- docs(readme): overhaul — add Quick Note, Notifications, App init, PGAP, Secrets, CLI reference sections; keep under-construction header with pre-v1 note (#1154) (#1159)
- feat(terminal): new_window layout — parallel dispatch spawns windows not splits (#1158)
- fix(close): advance focus to next sibling on pane/page close (#1152) (#1155)
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.13] — 2026-05-12

### Changes
- fix(close): advance focus to next sibling on pane/page close (#1152) (#1155)
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.12] — 2026-05-12

### Changes
- feat(quick-note): add clipboard destination to default config
- ux(quick-note): compose TextEdit fills 80% screen height on open (#1127) (#1151)
## [3.6.11] — 2026-05-12

### Changes
- fix(process-app): immediate repaint after Click eliminates ~100ms button lag (#1134) (#1136)
- fix(workspace): new page follows active pane CWD, context root is fallback only (#1140)
- sec(quick-note): audit shell injection for pane destination token substitution (#1113) (#1138)
- ux(modals): polish text inputs — cursor width, accent color, border feedback, margin (#1108) (#1137)
- feat(quick-note): visual polish & keyboard navigation (#1122) (#1124)
- docs: add DEV_LOG entry for PR #1117 config template
- feat(config): all 6 theme presets + docs link in generated config (#1114) (#1117)
- feat(navigation): pane focus history — Cmd+[ / Cmd+] time-travel through recently focused panes (#1087) (#1119)
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.10] — 2026-05-12

### Changes
- sec(quick-note): audit shell injection for pane destination token substitution (#1113) (#1138)
- ux(modals): polish text inputs — cursor width, accent color, border feedback, margin (#1108) (#1137)
- feat(quick-note): visual polish & keyboard navigation (#1122) (#1124)
- docs: add DEV_LOG entry for PR #1117 config template
- feat(config): all 6 theme presets + docs link in generated config (#1114) (#1117)
- feat(navigation): pane focus history — Cmd+[ / Cmd+] time-travel through recently focused panes (#1087) (#1119)
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.9] — 2026-05-12

### Changes
- sec(quick-note): audit shell injection for pane destination token substitution (#1113) (#1138)
- ux(modals): polish text inputs — cursor width, accent color, border feedback, margin (#1108) (#1137)
- feat(quick-note): visual polish & keyboard navigation (#1122) (#1124)
- docs: add DEV_LOG entry for PR #1117 config template
- feat(config): all 6 theme presets + docs link in generated config (#1114) (#1117)
- feat(navigation): pane focus history — Cmd+[ / Cmd+] time-travel through recently focused panes (#1087) (#1119)
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.8] — 2026-05-11

### Changes
- feat(quick-note): visual polish & keyboard navigation (#1122) (#1124)
- docs: add DEV_LOG entry for PR #1117 config template
- feat(config): all 6 theme presets + docs link in generated config (#1114) (#1117)
- feat(navigation): pane focus history — Cmd+[ / Cmd+] time-travel through recently focused panes (#1087) (#1119)
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.7] — 2026-05-11

### Changes
- docs: add DEV_LOG entry for PR #1117 config template
- feat(config): all 6 theme presets + docs link in generated config (#1114) (#1117)
- feat(navigation): pane focus history — Cmd+[ / Cmd+] time-travel through recently focused panes (#1087) (#1119)
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.6] — 2026-05-11

### Changes
- feat(navigation): pane focus history — Cmd+[ / Cmd+] time-travel through recently focused panes (#1087) (#1119)
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.5] — 2026-05-11

### Changes
- fix(rename-pane): one-shot focus guard, immediate sync, and overlay modal gate (#1118)
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.4] — 2026-05-11

### Changes
- feat(navigation): remap tab cycle to Cmd+Shift+H/L, add first/last Cmd+Shift+J/K (#1102) (#1107)
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.3] — 2026-05-11

### Changes
- fix(mcp-renderer): reduce handshake timeout to 10s, surface server stderr (#1050) (#1106)
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.2] — 2026-05-11

### Changes
- chore(skills): ship-issue skill update
- feat(quick-note): backend — FocusLayer modal, config routing, submenu (#1105)
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.1] — 2026-05-11

### Changes
- chore(skills): add bundle mode to ship-issue skill
- fix(examples): normalize Enter/Escape key strings in mcp-renderer and descriptor-renderer (#1103)
## [3.6.0] — 2026-05-11

### Changes
- fix(sdk): update mypy type-ignore code from method-assign to assignment
- fix(app-renderer): add padding/inset to MCP and CLI app panes (#1097)
- fix(app-init): remove ctx.notify() from default app scaffold (#1094)
- polish(sdk): reduce badge() default radius from RADIUS_MD to 6.0 (#1096)
- feat(pgap): declarative layout model — flex/stack/row trees (#1090)
- feat(dev): auto-restart watched apps after crash with 2s delay (#1055) (#1092)
- fix(sdk): hold button active_fill for 150ms after click (#1083) (#1085)
- feat(examples): gh-issues — GitHub Issues viewer app (#1081)
- polish(examples): migrate mcp-demo, backlog, csv_viewer to SelectList/FormField (#1071)
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.136] — 2026-05-11

### Changes
- fix(app-renderer): add padding/inset to MCP and CLI app panes (#1097)
- fix(app-init): remove ctx.notify() from default app scaffold (#1094)
- polish(sdk): reduce badge() default radius from RADIUS_MD to 6.0 (#1096)
- feat(pgap): declarative layout model — flex/stack/row trees (#1090)
- feat(dev): auto-restart watched apps after crash with 2s delay (#1055) (#1092)
- fix(sdk): hold button active_fill for 150ms after click (#1083) (#1085)
- feat(examples): gh-issues — GitHub Issues viewer app (#1081)
- polish(examples): migrate mcp-demo, backlog, csv_viewer to SelectList/FormField (#1071)
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.135] — 2026-05-11

### Changes
- fix(app-init): remove ctx.notify() from default app scaffold (#1094)
- polish(sdk): reduce badge() default radius from RADIUS_MD to 6.0 (#1096)
- feat(pgap): declarative layout model — flex/stack/row trees (#1090)
- feat(dev): auto-restart watched apps after crash with 2s delay (#1055) (#1092)
- fix(sdk): hold button active_fill for 150ms after click (#1083) (#1085)
- feat(examples): gh-issues — GitHub Issues viewer app (#1081)
- polish(examples): migrate mcp-demo, backlog, csv_viewer to SelectList/FormField (#1071)
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.134] — 2026-05-11

### Changes
- feat(pgap): declarative layout model — flex/stack/row trees (#1090)
- feat(dev): auto-restart watched apps after crash with 2s delay (#1055) (#1092)
- fix(sdk): hold button active_fill for 150ms after click (#1083) (#1085)
- feat(examples): gh-issues — GitHub Issues viewer app (#1081)
- polish(examples): migrate mcp-demo, backlog, csv_viewer to SelectList/FormField (#1071)
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.133] — 2026-05-11

### Changes
- fix(sdk): hold button active_fill for 150ms after click (#1083) (#1085)
- feat(examples): gh-issues — GitHub Issues viewer app (#1081)
- polish(examples): migrate mcp-demo, backlog, csv_viewer to SelectList/FormField (#1071)
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.132] — 2026-05-11

### Changes
- feat(examples): gh-issues — GitHub Issues viewer app (#1081)
- polish(examples): migrate mcp-demo, backlog, csv_viewer to SelectList/FormField (#1071)
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.131] — 2026-05-11

### Changes
- feat(pane-ops): Cmd+Ctrl+H/L moves pane into adjacent window at boundary (#1078)
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.130] — 2026-05-11

### Changes
- feat(nav): Cmd+K/J at vertical boundary wraps within context (#1077)
- feat(sdk): restyle default app init screen (#1073)
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.129] — 2026-05-11

### Changes
- feat(sdk): add on_text_submitted hook (#1067) (#1072)
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.128] — 2026-05-11

### Changes
- fix(cli): app init splits origin pane, not focused pane (#1068)
- feat(sdk): add ListView widget to plexi_sdk.widgets (#1069)
- feat(apps): add keyboard-driven Kanban app
- chore: update dev log
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.127] — 2026-05-11

### Changes
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.126] — 2026-05-11

### Changes
- fix(examples/timer): add 15s duration + global notification scope (#1063)
- polish(renderers): migrate mcp-renderer + descriptor-renderer to plexi_sdk.ui (#1061)
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.125] — 2026-05-11

### Changes
- chore: restore session edits carried through merge
- fix(sdk+timer): key aliases + timer ring rendering (#1060)
- chore: commit alpha changes before branching
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.124] — 2026-05-11

### Changes
- feat(examples): timer app with J/K duration and custom notifications (#1056)
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.124] — 2026-05-11

### Changes
- fix(sdk): always re-seed SDK on launch; add AppBar subtitle support (#1054)
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.123] — 2026-05-11

### Changes
- fix(mcp-renderer): augment PATH when spawning MCP server subprocess (#1053)
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.122] — 2026-05-11

### Changes
- feat(examples): migrate csv_viewer file list to ctx.list_view() (#1052)
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.121] — 2026-05-11

### Changes
- feat(cli): improve app init Python template with SDK primitives (#1049)
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.120] — 2026-05-11

### Changes
- feat(examples): csv_viewer — browse and inspect CSVs in launch directory (#1046)
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.119] — 2026-05-11

### Changes
- feat(cli): --mcp and --cli flags on plexi open (#1033, #1034) (#1048)
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.118] — 2026-05-11

### Changes
- fix(deps): disable eframe accesskit feature to eliminate accesskit_consumer panic (#1039)
- feat(cli): cli_help_parser — parse_help_to_descriptor (#1042)
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.117] — 2026-05-11

### Changes
- example(mcp-renderer): add demo_server.py — local MCP server for renderer testing (#1044)
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.116] — 2026-05-11

### Changes
- fix(mcp-renderer): use plexi open syntax in no-command error message (#1041)
- chore: commit alpha changes before branching
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.115] — 2026-05-11

### Changes
- feat(quick-note): destination picker UI stub (#1030)
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.114] — 2026-05-11

### Changes
- fix(file-browser): verify Enter/l/→ open files via system opener (#138) (#1029)
- feat(navigation): Cmd+HJKL falls through to window nav at pane boundary (#1017)
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.114] — 2026-05-11

### Changes
- fix(keybindings): Cmd+R rename-pane and font-size bindings broken by exact-modifier guard (#1027)
- feat(logging): date-based log rotation with configurable retention (#1028)
- chore: update ship-issue skill
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.113] — 2026-05-11

### Changes
- feat: app_states rename, pane key injection, file-browser no-repeat nav (#1026)
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.112] — 2026-05-11

### Changes
- feat(notify): snooze choice — defer notification by N seconds (#1024)
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.112] — 2026-05-11

### Changes
- feat(cli): inject PLEXI_CONTEXT_ID/NAME env vars + context current command (#1025)
- fix(bundle): keyboard_capture for input-inspector + rename app_state dir (#907, #851) (#1023)
- chore: auto-install after promoting to main
## [3.5.111] — 2026-05-10

### Changes
- fix(palette): surrender palette_search focus on dismiss to prevent AccessKit stale node crash (#1018) (#1020)
- fix(typing-tutor): correct key name strings to match host delivery format (#1016)
- chore: skill edit carried through merge
- chore(examples): remove voice-agent example app (#1015)
- chore: skill edit carried through merge
- feat(cli): reinstate `plexi app install <path>` command (#1014)
- feat: transparent titlebar with fullsize content view (#1013)
## [3.5.110] — 2026-05-10

### Changes
- fix(typing-tutor): correct key name strings to match host delivery format (#1016)
- chore: skill edit carried through merge
- chore(examples): remove voice-agent example app (#1015)
- chore: skill edit carried through merge
- feat(cli): reinstate `plexi app install <path>` command (#1014)
- feat: transparent titlebar with fullsize content view (#1013)
## [3.5.109] — 2026-05-10

### Changes
- chore(examples): remove voice-agent example app (#1015)
- chore: skill edit carried through merge
- feat(cli): reinstate `plexi app install <path>` command (#1014)
- feat: transparent titlebar with fullsize content view (#1013)
## [3.5.108] — 2026-05-10

### Changes
- chore: skill edit carried through merge
- feat(cli): reinstate `plexi app install <path>` command (#1014)
- feat: transparent titlebar with fullsize content view (#1013)
## [3.5.107] — 2026-05-10

### Changes
- feat: transparent titlebar with fullsize content view (#1013)
## [3.5.106] — 2026-05-10

### Changes
- skill(orchestrator): plexi-orchestrator — spawn parallel Claude ship sessions with auto-layout (#1012)
## [3.5.105] — 2026-05-10

### Changes
- feat(cli): plexi app link/unlink — register local app dirs with workspace (#984)
- feat(examples): recover balls physics demo (#1007)
- chore: commit alpha changes before branching
- chore: restore ship-issue skill edit
- chore: restore session edits carried through merge
- fix(notify): pane list only returns panes with tiles so pane_focus always succeeds (#1005)
- chore: update skills to use pane name instead of pane set-title
- improve: add cfg(unix) propagation lesson to CLAUDE.md
- feat(cli): rename pane set-title → pane name (#800) (#1001)
- chore: restore session edits carried through merge
- feat(notify): add --scope flag to plexi notify CLI (window|context|global) (#1002)
- fix(registry): launch .py app entries via python3 — remove shebang and chmod+x requirements (#1004)
- feat(secrets): add plexi secret get to read secret value to stdout (#1000)
- feat(create-app): scaffold + open + hot-reload skill (#1003)
- fix(open): use registry layout_hint when no --layout passed to plexi open (#999)
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.104] — 2026-05-10

### Changes
- chore: restore ship-issue skill edit
- chore: restore session edits carried through merge
- fix(notify): pane list only returns panes with tiles so pane_focus always succeeds (#1005)
- chore: update skills to use pane name instead of pane set-title
- improve: add cfg(unix) propagation lesson to CLAUDE.md
- feat(cli): rename pane set-title → pane name (#800) (#1001)
- chore: restore session edits carried through merge
- feat(notify): add --scope flag to plexi notify CLI (window|context|global) (#1002)
- fix(registry): launch .py app entries via python3 — remove shebang and chmod+x requirements (#1004)
- feat(secrets): add plexi secret get to read secret value to stdout (#1000)
- feat(create-app): scaffold + open + hot-reload skill (#1003)
- fix(open): use registry layout_hint when no --layout passed to plexi open (#999)
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.103] — 2026-05-10

### Changes
- feat(cli): rename pane set-title → pane name (#800) (#1001)
- chore: restore session edits carried through merge
- feat(notify): add --scope flag to plexi notify CLI (window|context|global) (#1002)
- fix(registry): launch .py app entries via python3 — remove shebang and chmod+x requirements (#1004)
- feat(secrets): add plexi secret get to read secret value to stdout (#1000)
- feat(create-app): scaffold + open + hot-reload skill (#1003)
- fix(open): use registry layout_hint when no --layout passed to plexi open (#999)
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.102] — 2026-05-10

### Changes
- chore: restore session edits carried through merge
- feat(notify): add --scope flag to plexi notify CLI (window|context|global) (#1002)
- fix(registry): launch .py app entries via python3 — remove shebang and chmod+x requirements (#1004)
- feat(secrets): add plexi secret get to read secret value to stdout (#1000)
- feat(create-app): scaffold + open + hot-reload skill (#1003)
- fix(open): use registry layout_hint when no --layout passed to plexi open (#999)
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.101] — 2026-05-10

### Changes
- feat(create-app): scaffold + open + hot-reload skill (#1003)
- fix(open): use registry layout_hint when no --layout passed to plexi open (#999)
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.100] — 2026-05-10

### Changes
- fix(open): use registry layout_hint when no --layout passed to plexi open (#999)
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.99] — 2026-05-10

### Changes
- fix(ship-skill): add plexi app init scaffold rule for test fixtures
- feat(pane): show pending notification badge in app pane chrome (#997)
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.98] — 2026-05-10

### Changes
- feat(crash-overlay): press C to copy full crash report to clipboard (#986) (#989)
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.97] — 2026-05-10

### Changes
- fix(pane-ops): delete zombie window when last pane closes via IPC (#995)
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.96] — 2026-05-10

### Changes
- feat(notify): add --host-action flag for fire-and-forget choice actions (#991)
- fix(ship-skill): delete test script in Phase 5 cleanup
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.95] — 2026-05-10

### Changes
- fix(zoom): clear zoom overlay when launching a non-overlay app (#920) (#993)
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.94] — 2026-05-10

### Changes
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.93] — 2026-05-10

### Changes
- fix(zoom): clear accesskit focus on zoom overlay transition to prevent panic (#988)
- feat(config): user-configurable keybindings in config.toml (#985)
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.92] — 2026-05-10

### Changes
- chore: commit alpha changes before branching
- chore: add shell_join single-arg gotcha to GOTCHAS.md
- improve: add shell suffix construction lesson to CLAUDE.md
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.91] — 2026-05-10

### Changes
- feat(cli): plexi terminal <cmd> stays alive by default; --ephemeral opts into exit-on-complete (#981)
- chore: skill state from ship session
- feat(cli): add --from-pane-id to plexi terminal and plexi open (#975)
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.90] — 2026-05-10

### Changes
- chore: skill state from ship session
- docs(pgap): PGAP protocol v1 reference — DrawCommands, Capabilities, stability markers (#976)
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.89] — 2026-05-10

### Changes
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.88] — 2026-05-10

### Changes
- chore: bundle small fixes (#814, #682, #743, #809, #842) (#919)
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.87] — 2026-05-10

### Changes
- fix(hot-reload): activate based on manifest flag, not discovery path (#971)
- feat(ship-skill): require [AI REVIEW] output block after Phase 4b — surface CodeRabbit and Gemini findings with per-item dispositions
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.86] — 2026-05-10

### Changes
- chore: ship-issue skill update
- chore(examples): gut dev-artifact example apps, remove shell-init (#956)
- fix(sdk): auto-close orphaned pipe on audio_capture restart; add stop_audio_capture() (#952)
- fix(ship-skill): dirty check catches deletions and renames, not just modifications
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.85] — 2026-05-10

### Changes
- refactor(notifications): replace source_context index with stable source_context_id (#967)
- chore: fix ship skill Phase 5 — stash+reset instead of rebase to prevent squash conflicts
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.84] — 2026-05-10

### Changes
- fix(zoom): render name bar outside Frame so terminal bg can't cover it (#964)
- feat(mcp): PGAP↔MCP bridge — consume any MCP server as a Plexi app; expose any Plexi app as an MCP server (#965)
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.83] — 2026-05-10

### Changes
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.82] — 2026-05-10

### Changes
- fix(cli): shorten secret set description in parent command listing
- fix(cli): plexi pane send works across all windows (#955)
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.81] — 2026-05-10

### Changes
- fix(cli): shorten secret set description in parent command listing
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.80] — 2026-05-10

### Changes
- feat(secrets): safe CLI — hidden prompt, walk-up scoping, root guard (#953)
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.79] — 2026-05-10

### Changes
- fix(sidebar): always show notification badge regardless of hover state (#963)
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.78] — 2026-05-10

### Changes
- fix(background-apps): notifications route to park-time context (#957)
- chore: update ship-issue skill
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.77] — 2026-05-09

### Changes
- fix(examples/typing-tutor): apply Gemini review fixes from PR #936 (#948)
- improve: broaden git worktree lesson to all git -C operations
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.76] — 2026-05-09

### Changes
- feat(registry): add CLI descriptors for git, docker, kubectl, brew, uv, rg, fzf (#942)
- chore: skill fixup
- chore: ship-issue skill update
- chore: remove dead agent infrastructure (#934) (#943)
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.75] — 2026-05-09

### Changes
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.74] — 2026-05-09

### Changes
- improve: add PLEXI_SOCKET blocks PR build launch to GOTCHAS.md
- feat(examples): add typing-tutor app with 10 progressive levels and star gating (#936)
- feat(sdk): App lazy-init guard + AppHarness headless subprocess runner (#870 #865) (#925)
- fix(permissions): corrupt-file backup + prefix-collision bleed (#875, #874) (#924)
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.73] — 2026-05-09

### Changes
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.72] — 2026-05-09

### Changes
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.71] — 2026-05-09

### Changes
- chore: commit alpha changes before branching
- feat(cli): plexi pane close — no-arg form closes current pane via PLEXI_PANE_ID (#896)
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.70] — 2026-05-09

### Changes
- chore: update ship-issue skill
- fix(process-app): suppress duplicate on_key calls for printable non-letter keys (#935)
- chore: commit skill changes before branching
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.69] — 2026-05-09

### Changes
- refactor(file-browser): centralize key dispatch via FileBrowserAction + classify_key (#928)
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.68] — 2026-05-09

### Changes
- fix(file-browser): prefix PTY cd write with Ctrl-U to clear readline buffer (#923)
- chore: remove spec doc — issue #920 is the deliverable
- feat(file-browser): open as overlay + sync CWD back to terminal on close (#916)
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.67] — 2026-05-09

### Changes
- chore: remove spec doc — issue #920 is the deliverable
- improve: ship-issue skill — test discrimination, CLI self-test, cr CLI fix
- docs: app-launch-clears-zoom design spec
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.66] — 2026-05-09

### Changes
- feat(agent): voice-agent PGAP app — mic → Whisper → ai.query → TTS (#900)
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.65] — 2026-05-09

### Changes
- fix(sdk): ctx.clear() default fill (#000000), add width/height aliases (#871) (#911)
- security(cli-crawl): sanitize CLI name before using as cache filename (#910)
- fix(updater): strip pre-release suffixes in semver_gt, add 5s HTTP timeout (#909)
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.64] — 2026-05-09

### Changes
- fix(cli): shell_join preserves quoted multi-word args for terminal -c (#915)
- chore(ship-skill): add Python test script rule for interactive PR testing
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.63] — 2026-05-09

### Changes
- feat(cli): notify --choice accepts 4-segment key:Label:action:arg format (#914)
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.62] — 2026-05-09

### Changes
- feat(cli): pipe pane list/info JSON through jq when available (#906)
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.61] — 2026-05-09

### Changes
- feat(apps): default launch layout to overlay instead of split_v (#904)
- feat(ship): add AI review phase + PR number in pane title
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.60] — 2026-05-09

### Changes
- fix(cli): restore --ephemeral support after PR #890 removed close_on_exit field
- fix: update close_on_exit → ephemeral after TerminalPane field rename
- chore: commit alpha changes before branching
- improve: HostHarness add_test_pane() creates App pane — doc + lesson
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.59] — 2026-05-09

### Changes
- fix(palette): Cmd+P bypasses keyboard capture so palette always opens (#899)
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.58] — 2026-05-09

### Changes
- feat(cli): promote plexi terminal as top-level command (#901)
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.57] — 2026-05-09

### Changes
- chore: commit alpha changes before branching
- fix(terminal): plexi open terminal <cmd> panes persist on process exit (#890) (#898)
- chore: update ship-issue skill — pane capture, Cargo.toml restore, end-of-run notify
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.56] — 2026-05-09

### Changes
- feat(cli): pane set-title accepts optional pane ID — plexi pane set-title [id] <title> (#897)
- docs: add parse_workspace_path_arg subcommands gotcha
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.55] — 2026-05-09

### Changes
- feat(snake): increase grid to 26×20 with 18px cells (#887)
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.54] — 2026-05-09

### Changes
- fix(pane): closing a pane focuses the pane to the right (or below) (#895)
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.53] — 2026-05-09

### Changes
- feat(cli): plexi pane info — dump current pane context as JSON (#894)
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.52] — 2026-05-09

### Changes
- feat(cli): pane list — add context_id, context_name, window_id, cwd fields (#893)
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.51] — 2026-05-08

### Changes
- fix(snake-race): lower combined target from 100 to 50 (#886)
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.50] — 2026-05-08

### Changes
- feat: snake-race — four-pane speedrun to 100 with timer and personal best (#883)
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.49] — 2026-05-08

### Changes
- feat(snake): emit score and game_over events via pipe (#882)
- docs: add env setup gotcha for Plexi app subprocess spawning
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.48] — 2026-05-08

### Changes
- feat(cli): plexi app render — headless app screenshot via offscreen egui/wgpu (#873)
- chore: stop ignoring sdk/python/tests/ — test files should be tracked
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.47] — 2026-05-08

### Changes
- Rename ship skill to ship-issue
- Add Plexi-specific ship cycle gotchas to ship skill
- fix(sdk): raise TypeError when blocking emit called from sync hook (#872)
- chore: commit alpha changes before branching
## [3.5.46] — 2026-05-08

### Changes
- feat(permissions): add PermissionStore + three-state model (#860)
- chore: update ship skill
- feat(sdk): button primitive with hover, focus, and keyboard activation (#857)
- chore: add closed-issue guard to ship skill Phase 1
- fix(docs): mark pseudocode doctest as ignore to fix cargo test (#859)
- fix(logging): demote per-app AppRegistry load logs from info to debug (#856)
- fix(sdk): expose from_pane_id + request_id on RenderContext.spawn_pane() (#752) (#855)
- fix(context): use router.active_idx() for rename target instead of active_window (#854)
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.45] — 2026-05-08

### Changes
- chore: update ship skill
- feat(sdk): button primitive with hover, focus, and keyboard activation (#857)
- chore: add closed-issue guard to ship skill Phase 1
- fix(docs): mark pseudocode doctest as ignore to fix cargo test (#859)
- fix(logging): demote per-app AppRegistry load logs from info to debug (#856)
- fix(sdk): expose from_pane_id + request_id on RenderContext.spawn_pane() (#752) (#855)
- fix(context): use router.active_idx() for rename target instead of active_window (#854)
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.44] — 2026-05-08

### Changes
- chore: add closed-issue guard to ship skill Phase 1
- fix(docs): mark pseudocode doctest as ignore to fix cargo test (#859)
- fix(logging): demote per-app AppRegistry load logs from info to debug (#856)
- fix(sdk): expose from_pane_id + request_id on RenderContext.spawn_pane() (#752) (#855)
- fix(context): use router.active_idx() for rename target instead of active_window (#854)
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.43] — 2026-05-08

### Changes
- chore: add closed-issue guard to ship skill Phase 1
- fix(logging): demote per-app AppRegistry load logs from info to debug (#856)
- fix(sdk): expose from_pane_id + request_id on RenderContext.spawn_pane() (#752) (#855)
- fix(context): use router.active_idx() for rename target instead of active_window (#854)
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.42] — 2026-05-08

### Changes
- fix(sdk): expose from_pane_id + request_id on RenderContext.spawn_pane() (#752) (#855)
- fix(context): use router.active_idx() for rename target instead of active_window (#854)
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.41] — 2026-05-08

### Changes
- fix(context): use router.active_idx() for rename target instead of active_window (#854)
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.40] — 2026-05-08

### Changes
- improve(ship): add Phase 1b implementation audit step to check if work already landed on alpha
- test(notify): HostHarness tests for pane_navigate and DeliverNotifyAction dispatch (#791, #823) (#858)
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.39] — 2026-05-08

### Changes
- fix(minimap): anchor overlay Area at panel_min to restore sidebar hover state (#853)
- docs: add CLI namespace design guidance
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.38] — 2026-05-08

### Changes
- feat(sdk): workspace-scoped app state with walk-up resolution (#834) (#838)
- fix(install): derive channel from git branch instead of .channel file
- improve: add git worktree staging lesson to CLAUDE.md
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.37] — 2026-05-08

### Changes
- feat(pane): plexi open returns new pane ID + plexi pane send injects text (#843)
- chore: commit alpha changes before branching
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.36] — 2026-05-08

### Changes
- feat(descriptor): warn when installed app shadows same-named CLI plexi_app (#836)
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.35] — 2026-05-08

### Changes
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.34] — 2026-05-08

### Changes
- chore: stash ship skill changes before merge
- feat(spawn_pane): add from_pane_id + request_id correlation (#830)
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.33] — 2026-05-08

### Changes
- chore: update ship skill — test fixture rule
- refactor(cli): remove plexi app install — consolidate to plexi install (#828)
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.32] — 2026-05-08

### Changes
- fix(notify): sync router active index in pane_navigate so sidebar updates (#829)
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.31] — 2026-05-08

### Changes
- feat(descriptor): add plexi_app field — let CLIs declare a custom PGAP app entry point (#831)
- improve: add cargo test binary target lesson to GOTCHAS.md
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.30] — 2026-05-08

### Changes
- feat(manifest): notification_scope — window, context, global per app (#827)
- chore: improve ship skill Phase 5 CWD guidance
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.29] — 2026-05-07

### Changes
- feat(secrets): safe CLI for plexi secret set — hidden prompt, walk-up scoping, root guard (#818)
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.28] — 2026-05-07

### Changes
- chore: apply CLAUDE.md linter update
- fix(notify): host-side pane_navigate + cross-context action dispatch (#819)
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.27] — 2026-05-07

### Changes
- fix(cli): honest help text for shell-init no-op and pane-only commands (#817)
- docs: add bundle label definition to CLAUDE.md
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.26] — 2026-05-07

### Changes
- chore(ship): add log verification rule — agent reads logs before surfacing testing block
- refactor(secrets): replace security CLI subprocess with security-framework crate (#783)
- fix(typed_pipes): detect drain thread broken pipe within one render frame (#763)
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.25] — 2026-05-07

### Changes
- chore: update workflow for alpha-as-root layout (#808)
## [3.5.24] — 2026-05-07

### Changes
- feat(just): add uninstall command (#805)
- chore(just): remove next-issue recipe (#806)
- docs(claude): document completions testing gap for PR builds
## [3.5.23] — 2026-05-07

### Changes
- fix(context): strip CWD auto-switch shell hook + neutralise FocusContext (#796) (#798)
- chore(just): pr-install auto-cleans previous build before reinstalling
- chore(skills): move ship skill into repo, remove triage-issues
- docs(claude): clean up stale CLAUDE.md — remove version labels, PlexiApp State, Terminology; point North Star to NORTH_STAR.md; document worktree subdirs
- fix(triage): inline GraphQL ID lookups to stay within Bash(gh *) tool allowlist
- chore(issues): auto-label new issues + modernize triage workflow
- chore(triage): add needs-info label + sharpen clarification step
- docs: add NORTH_STAR.md and wire triage-issues to it
## [3.5.22] — 2026-05-07

### Changes
- fix(install): correct app bundle name, icon, and shell-init job noise
- revert(context): remove CWD-based auto-switch — sidebar flashing bug (#793)
## [3.5.21] — 2026-05-07

### Changes
- revert(context): remove CWD-based auto-switch — sidebar flashing bug (#793)
## [3.5.20] — 2026-05-07

### Changes
- docs: use raw GitHub URL for install one-liner until plexiapp.com/install is live
## [3.5.19] — 2026-05-07

### Changes
- fix(first-run): re-prompt CLI setup on migrated profiles, always seed SDK (#792)
## [3.5.18] — 2026-05-06

### Changes
- feat(cli): add --version flag support (#788)
- feat(voice): voice.toml config with per-workspace overlay (#780)
- docs(claude): document build channels, isolated profiles, and PR build testing rules
- feat(calendar): month-view calendar app for PAM voice demo (#781)
- feat(cli): plexi completions <zsh|bash|fish> + auto-install on every install (#784)
- feat(context): auto-switch context on pane focus via host-side CWD watch (#717) (#782)
- feat(cli): add pane list, focus, close subcommands (#744) (#779)
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.17] — 2026-05-06

### Changes
- feat(voice): voice.toml config with per-workspace overlay (#780)
- docs(claude): document build channels, isolated profiles, and PR build testing rules
- feat(calendar): month-view calendar app for PAM voice demo (#781)
- feat(cli): plexi completions <zsh|bash|fish> + auto-install on every install (#784)
- feat(context): auto-switch context on pane focus via host-side CWD watch (#717) (#782)
- feat(cli): add pane list, focus, close subcommands (#744) (#779)
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.16] — 2026-05-06

### Changes
- feat(calendar): month-view calendar app for PAM voice demo (#781)
- feat(cli): plexi completions <zsh|bash|fish> + auto-install on every install (#784)
- feat(context): auto-switch context on pane focus via host-side CWD watch (#717) (#782)
- feat(cli): add pane list, focus, close subcommands (#744) (#779)
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.15] — 2026-05-06

### Changes
- feat(cli): plexi completions <zsh|bash|fish> + auto-install on every install (#784)
- feat(context): auto-switch context on pane focus via host-side CWD watch (#717) (#782)
- feat(cli): add pane list, focus, close subcommands (#744) (#779)
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.14] — 2026-05-06

### Changes
- feat(context): auto-switch context on pane focus via host-side CWD watch (#717) (#782)
- feat(cli): add pane list, focus, close subcommands (#744) (#779)
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.13] — 2026-05-06

### Changes
- feat(cli): add pane list, focus, close subcommands (#744) (#779)
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.12] — 2026-05-06

### Changes
- refactor(sidebar): replace RowResult with SidebarAction enum (#778)
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.11] — 2026-05-06

### Changes
- feat(pty): inject PLEXI_CHANNEL into PTY environment (#776)
- chore: rename alpha bundle to 'Plexi Alpha'
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.10] — 2026-05-06

### Changes
- feat(terminal): auto-close pane when initial_cmd process exits (#772)
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.9] — 2026-05-06

### Changes
- fix(minimap): preserve visibility when closing a window in same context (#773)
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.8] — 2026-05-06

### Changes
- chore: add DEV_LOG.md
- fix(zoom): block background resize handles when pane is zoomed (#771)
- docs(readme): add Apps section — install, manage, and build apps via CLI
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.7] — 2026-05-06

### Changes
- fix(tetris): remove keyboard_capture = true from manifest (#770)
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.6] — 2026-05-06

### Changes
- feat(cli): plexi open terminal <cmd> passthrough + just next-issue recipe (#765)
- chore: remove premature/abandoned example apps from binary
- docs(glossary): replace stale "Plexi IQ" with "IQ query" in agent pane definition
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.5] — 2026-05-06

### Changes
- fix(process_app): name all spawned threads + background reaper + stream thread cap (#758)
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.4] — 2026-05-06

### Changes
- fix(cli): atomic response file write + UUID filename in notify_cli (#755)
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.3] — 2026-05-06

### Changes
- fix(typed_pipes): replace yield_now() spin with 1ms sleep in drain loop (#756)
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.2] — 2026-05-06

### Changes
- fix(cli): plexi open terminal now opens a pane via socket and spawn-queue paths (#757)
- chore: commit skill changes before branching
- docs: remove voice agent spec file — content lives in GitHub issues
- docs: voice agent design spec
- refactor: migrate issue dependencies from front-matter to native GitHub blocking
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.1] — 2026-05-06

### Changes
- fix(cli): stop PLEXI_RUNNING walk before home dir to avoid reporting stable profile as workspace (#740)
- refactor: replace DEV_LOG with GOTCHAS.md + detailed commit messages
- docs: GOTCHAS.md + ship skill git flow redesign spec
## [3.5.0] — 2026-05-06

### Changes
## [3.4.120] — 2026-05-05

### Changes
- chore: log PR #737 channel-agnostic CLI fix
- feat: update CLI tagline to "the last app you'll ever need" (#739)
- fix(cli): socket-first open_cli; install.sh keeps bare plexi in sync (#737)
- docs(cli_setup): note that just pr-install pre-creates the symlink (#736)
- fix(onboarding): CLI setup modal shows error and stays open on symlink failure (#735)
- fix(updater): only notify when latest > current semver (#733)
- fix(install): embed plexi_sdk, seed to profile dir on first launch (#734)
- fix(promote): read EOF under set -e exits 1 — add || true
## [3.4.119] — 2026-05-05

### Changes
- chore: log PR #737 channel-agnostic CLI fix
- fix(cli): socket-first open_cli; install.sh keeps bare plexi in sync (#737)
- docs(cli_setup): note that just pr-install pre-creates the symlink (#736)
- fix(onboarding): CLI setup modal shows error and stays open on symlink failure (#735)
- fix(updater): only notify when latest > current semver (#733)
- fix(install): embed plexi_sdk, seed to profile dir on first launch (#734)
- fix(promote): read EOF under set -e exits 1 — add || true
## [3.4.118] — 2026-05-05

### Changes
- docs(cli_setup): note that just pr-install pre-creates the symlink (#736)
- fix(onboarding): CLI setup modal shows error and stays open on symlink failure (#735)
- fix(updater): only notify when latest > current semver (#733)
- fix(install): embed plexi_sdk, seed to profile dir on first launch (#734)
- fix(promote): read EOF under set -e exits 1 — add || true
## [3.4.117] — 2026-05-05

### Changes
- fix(onboarding): CLI setup modal shows error and stays open on symlink failure (#735)
- fix(updater): only notify when latest > current semver (#733)
- fix(install): embed plexi_sdk, seed to profile dir on first launch (#734)
- fix(promote): read EOF under set -e exits 1 — add || true
## [3.4.116] — 2026-05-05

### Changes
- fix(updater): only notify when latest > current semver (#733)
- fix(install): embed plexi_sdk, seed to profile dir on first launch (#734)
- fix(promote): read EOF under set -e exits 1 — add || true
## [3.4.115] — 2026-05-05

### Changes
- fix(install): embed plexi_sdk, seed to profile dir on first launch (#734)
- fix(promote): read EOF under set -e exits 1 — add || true
## [3.4.114] — 2026-05-05

### Changes
- fix(release-ci): include Cargo.toml in cache key + always clean bundle output (#732)
- feat(tiling): widen active pane focus outline to fill inter-pane gap (#722)
- feat(keys): delete RunPalette, bind Cmd+R → rename pane, Cmd+Shift+R → rename context (#720)
## [3.4.113] — 2026-05-05

### Changes
- feat(tiling): widen active pane focus outline to fill inter-pane gap (#722)
- feat(keys): delete RunPalette, bind Cmd+R → rename pane, Cmd+Shift+R → rename context (#720)
## [3.4.112] — 2026-05-05

### Changes
- feat(keys): delete RunPalette, bind Cmd+R → rename pane, Cmd+Shift+R → rename context (#720)
## [3.4.111] — 2026-05-05

### Changes
- fix(release): bundle Python runtime in release zip (#718)
- refactor(cli): migrate CLI dispatch to clap — structured subcommands, --help (#714)
- feat(context): configurable context root — project container foundation (#709)
- fix(tiling): remove idle divider stroke to fix white lines in light mode (#707)
## [3.4.110] — 2026-05-05

### Changes
- refactor(cli): migrate CLI dispatch to clap — structured subcommands, --help (#714)
- feat(context): configurable context root — project container foundation (#709)
- fix(tiling): remove idle divider stroke to fix white lines in light mode (#707)
## [3.4.109] — 2026-05-05

### Changes
- feat(context): configurable context root — project container foundation (#709)
- fix(tiling): remove idle divider stroke to fix white lines in light mode (#707)
## [3.4.108] — 2026-05-05

### Changes
- fix(tiling): remove idle divider stroke to fix white lines in light mode (#707)
## [3.4.107] — 2026-05-05

### Changes
- fix(release): use git-cliff --latest for release notes (#708)
## [3.4.106] — 2026-05-05

### Changes
- polish(notify): wider modal, centered key hint row, separator footer (#704)
- improve: add issue-referenced code validation lesson to CLAUDE.md
- feat(notifications): strip reserved nav shortcuts from NotifyOption at host ingestion (#702)
- refactor(notify): migrate plexi notify CLI off file queue onto PLEXI_SOCKET (#701)
- feat(background-apps): tick parked apps + command palette bg indicator (#700)
- fix: set bundle name to Plexi for correct Dock/Spotlight display
- fix(install): find bundle dynamically, don't hardcode app name
## [3.4.105] — 2026-05-05

### Changes
- feat(notifications): strip reserved nav shortcuts from NotifyOption at host ingestion (#702)
- refactor(notify): migrate plexi notify CLI off file queue onto PLEXI_SOCKET (#701)
- feat(background-apps): tick parked apps + command palette bg indicator (#700)
- fix: set bundle name to Plexi for correct Dock/Spotlight display
- fix(install): find bundle dynamically, don't hardcode app name
## [3.4.104] — 2026-05-05

### Changes
- refactor(notify): migrate plexi notify CLI off file queue onto PLEXI_SOCKET (#701)
- feat(background-apps): tick parked apps + command palette bg indicator (#700)
- fix: set bundle name to Plexi for correct Dock/Spotlight display
- fix(install): find bundle dynamically, don't hardcode app name
## [3.4.103] — 2026-05-05

### Changes
- feat(background-apps): tick parked apps + command palette bg indicator (#700)
- fix: set bundle name to Plexi for correct Dock/Spotlight display
- fix(install): find bundle dynamically, don't hardcode app name
## [3.4.102] — 2026-05-05

### Changes
- improve: add inversion comment to split_focused LinearDir mapping
- feat(git-app): merge arc for squash PRs + fix edge routing overlap (#694)
- feat(terminal): copy-mode — keyboard-driven scrollback selection (#603) (#696)
- feat(pane-ops): DrawCommand::SpawnPane — apps can request new terminal/app panes (#692)
- feat(ui): redesign keyboard shortcuts modal — sections, divider, full coverage, contact footer (#693)
- feat(palette): rank active-context panes above other contexts (#408) (#691)
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.101] — 2026-05-05

### Changes
- improve: add inversion comment to split_focused LinearDir mapping
- feat(git-app): merge arc for squash PRs + fix edge routing overlap (#694)
- feat(terminal): copy-mode — keyboard-driven scrollback selection (#603) (#696)
- feat(pane-ops): DrawCommand::SpawnPane — apps can request new terminal/app panes (#692)
- feat(ui): redesign keyboard shortcuts modal — sections, divider, full coverage, contact footer (#693)
- feat(palette): rank active-context panes above other contexts (#408) (#691)
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.100] — 2026-05-05

### Changes
- improve: add inversion comment to split_focused LinearDir mapping
- feat(pane-ops): DrawCommand::SpawnPane — apps can request new terminal/app panes (#692)
- feat(ui): redesign keyboard shortcuts modal — sections, divider, full coverage, contact footer (#693)
- feat(palette): rank active-context panes above other contexts (#408) (#691)
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.99] — 2026-05-05

### Changes
- feat(pane-ops): DrawCommand::SpawnPane — apps can request new terminal/app panes (#692)
- feat(ui): redesign keyboard shortcuts modal — sections, divider, full coverage, contact footer (#693)
- feat(palette): rank active-context panes above other contexts (#408) (#691)
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.98] — 2026-05-05

### Changes
- feat(ui): redesign keyboard shortcuts modal — sections, divider, full coverage, contact footer (#693)
- feat(palette): rank active-context panes above other contexts (#408) (#691)
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.97] — 2026-05-05

### Changes
- feat(palette): rank active-context panes above other contexts (#408) (#691)
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.96] — 2026-05-05

### Changes
- feat(cli-crawl): Tier 3 --help fallback descriptor renderer (#360) (#685)
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.95] — 2026-05-05

### Changes
- fix: offload audio/MIDI device enumeration to background thread (#688)
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.94] — 2026-05-05

### Changes
- feat(ipc): implement PLEXI_SOCKET listener and plexi pane set-title (#686)
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.93] — 2026-05-05

### Changes
- feat(chat-poc): copy buttons, in-flight input overlay, tool docstring (#684)
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.92] — 2026-05-05

### Changes
- fix(palette): launch app from welcome screen seeds tree root (#683)
- improve: add SDK proxy wrappers lesson to CLAUDE.md
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.91] — 2026-05-05

### Changes
- feat(notifications): timeout_secs/on_dismiss, tombstone, required-pinned (#291) (#679)
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.90] — 2026-05-05

### Changes
- feat(secrets): add inject toggle to new-secret form (#670)
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.89] — 2026-05-05

### Changes
- feat(sdk): normalize arrow key names before on_key dispatch (#677)
- feat(ux): show welcome screen instead of deleting context when last pane is closed (#678)
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.88] — 2026-05-05

### Changes
- feat(audio): playback, AudioMeter, emit.list_audio_devices, audio-recorder POC (#341) (#673)
- feat(protocol): add StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) (#671)
- fix(screen-time): spread session secs across 15-min buckets (#668)
- feat(ci): auto-triage new issues via Claude Code Action
## [3.4.87] — 2026-05-05

### Changes
- revert(sidebar): remove broken full-row double-click interact, restores single-click (#659)
- fix(install): remove deleted agent apps from core pack list (#656)
- feat(terminal): scrollback keyboard navigation (#658)
- fix(changelog): cd into worktree before git-cliff, skip chore:release commits, rebuild empty entries
- fix(sidebar): widen double-click rename hit box to full context row (#657)
## [3.4.86] — 2026-05-05

### Changes
- fix(install): remove deleted agent apps from core pack list (#656)
- feat(terminal): scrollback keyboard navigation (#658)
- fix(changelog): cd into worktree before git-cliff, skip chore:release commits, rebuild empty entries
- fix(sidebar): widen double-click rename hit box to full context row (#657)
## [3.4.85] — 2026-05-05

### Changes
- fix: correct mypy ignore code for sys.stdout.reconfigure
- fix: add missing README.md for plexi-sdk hatchling build
- fix: run SDK type check on main/beta/alpha pushes, not just sdk/python paths
- fix: add workflow_dispatch trigger to SDK type check
- fix: use uv venv instead of --system for SDK type check CI

## [3.4.84] — 2026-05-04

### Changes
- Enhance README with more details and formatting
- Delete .plexi directory
- Delete .superpowers directory
- improve: add North Star alignment audit to triage-issues skill
- improve: add cargo bundle --bin lesson to CLAUDE.md
- fix(install): move gen_schema to workspace, restore display name + icon for PR builds (#655)
- feat(sdk): typed command models, py.typed, and plexi validate preflight tool (#627) (#651)
- fix(workspace): new window/page inherits cwd from focused pane (#652)
- feat(updater): once-a-day cached update check with toolbar badge (#648)
- refactor(sdk): split plexi_sdk/__init__.py into focused modules (#649)

## [3.4.79] — 2026-05-04

### Changes
- docs(readme): overhaul for accuracy — features + roadmap, drop stale sections (#647)
- feat(tiling): keyboard pane swap Cmd+Ctrl+HJKL + edge pulse (#413) (#646)

## [3.4.77] — 2026-05-04

### Changes
- fix: BSD awk compat in release notes extraction (#643)

## [3.4.76] — 2026-05-04

### Changes
- fix: pass --bin plexi to cargo bundle in release workflow (#642)

## [3.4.75] — 2026-05-04

### Changes
- feat(promote): chain alpha→beta→main when promoting to main from alpha (#641)
- fix(ci): use lowercase plexi.app path in release workflow (#640)
- fix(promote): skip tag creation/push if already exists (#639)

## [3.4.72] — 2026-05-04

### Changes
- refactor(pipeline): clean up release pipeline (#638)
- feat(release): git-cliff changelog + explicit release-version command (#637)

## [3.4.71] — 2026-05-04 01:55 ET

### Changes
- feat(release): git-cliff changelog + explicit release-version command (#637)
- chore: promote to beta — v3.4.70
- Add app architecture brainstorm docs and build automation scripts
- improve: add cargo-bundle multi-bin and SDK import proxy lessons to CLAUDE.md

## [3.4.70] — 2026-05-04

### Changes
- Add app architecture brainstorm docs and build automation scripts
- improve: add cargo-bundle multi-bin and SDK import proxy lessons to CLAUDE.md
- feat(triage): rename triage→triage-issues, add touches/clarification_needed, add sprint-plan batch skill (#636)
- feat(sdk): Rust-owned canonical PGAP schema + generated Python protocol models (#634)
- feat: plexi install <id> resolves bare app IDs via plexi-registry (#633)

## [3.4.69] — 2026-05-04 01:13 ET

### Changes

## [3.4.68] — 2026-05-04 00:34 ET

### Changes
- chore: add just pr-clean-merged recipe to remove stale PR build artifacts
- feat(sdk): Rust-owned canonical PGAP schema + generated Python protocol models (#634)

## [3.4.67] — 2026-05-03 23:35 ET

### Changes
- feat: plexi install <id> resolves bare app IDs via plexi-registry (#633)
- chore: promote to beta — v3.4.66

## [3.4.66] — 2026-05-03

### Changes
- feat: zoom overlay at 88% opacity — scrim bleed-through (#572) (#629)
- perf(sdk): batch Python frame output and remove frame.clone() hot-path copy (#624) (#630)
- chore: update triage skill — P0 priority, depends_on front matter, blocked label handling
- chore(apps): remove stale iq.query POC apps, fix capability hint (#623) (#628)
- improve: add Issue Prior Attempts convention to CLAUDE.md
- chore: replace issue template with typed feature/bug/idea templates + depends_on front matter
- feat(palette): Cmd+J / Cmd+K as ArrowDown / ArrowUp aliases in command palette (#620)
- feat(ui): increase zoomed pane inset from 5px to 10px (#618)
- refactor(routing): split DrawCommand into RenderCommand + HostCommand + ControlCommand (#538) (#621)
- fix(config): confirm_close defaults to false, matching template and UX intent (#614)
- feat(ui): minimap at 75% opacity (#615)
- fix(config): confirm_close defaults to false, matching template and UX intent (#614)
- refactor(pane): kill Pane::Agent and Pane::AgentWorkspace (#523) (#612)
- refactor(pane): kill Pane::Agent and Pane::AgentWorkspace (#523) (#612)
- feat(cli): plexi update — detached relaunch when run from inside Plexi (#606)
- feat(cli): plexi update — detached relaunch when run from inside Plexi (#606)
- docs: scrollback navigation + copy-mode design spec (#602, #603)
- feat(cli): implement plexi update — binary self-update for stable channel (#594) (#601)
- docs: DEV_LOG PR #600 — plexi update namespace rename
- refactor(cli): reserve 'plexi update' for self-update; move app updates to 'plexi update apps' (#600)
- docs: DEV_LOG PR #598 — SpawnPane, plexi open CLI, SDK ctx.spawn_pane()
- docs: DEV_LOG PR #598 — SpawnPane, plexi open CLI, SDK ctx.spawn_pane()
- docs: DEV_LOG PR #596 — empty-state notification modal
- feat(spawn): DrawCommand::SpawnPane — plexi open CLI, SDK ctx.spawn_pane(), panes.spawn capability (#598)
- fix(notifications): Cmd+Shift+A always opens modal, empty state when queue is empty (#596)
- docs: DEV_LOG PR #586 — notify choice response file fix
- docs(lessons): command self-containment — handler data belongs in the command, not ambient state
- feat(cli): plexi notify --choice blocking flow (#586)
- docs: DEV_LOG PR #589 — defer heartbeat until after shell probes
- chore: replace mit label with P0 (won't fix) in priority scale
- fix(startup): defer heartbeat until after shell probes (#588) (#589)
- docs: require instrumentation for new features; document macOS file drag pointer state gotcha
- docs: DEV_LOG PR #585 — freeze + drop fix on zoomed pane
- fix(promote): correct changelog range + clean up corrupt 3.4.48 entry
- fix(drag): skip TerminalView render during file hover on zoomed pane (#585)
- chore: remove redundant skill callouts from CLAUDE.md; add triage YAML frontmatter

## [3.4.65] — 2026-05-03 23:20 ET

### Changes

## [3.4.64] — 2026-05-03 22:49 ET

### Changes
- perf(sdk): batch Python frame output and remove frame.clone() hot-path copy (#624) (#630)
- chore: update triage skill — P0 priority, depends_on front matter, blocked label handling

## [3.4.63] — 2026-05-03 22:39 ET

### Changes
- chore(apps): remove stale iq.query POC apps, fix capability hint (#623) (#628)
- improve: add Issue Prior Attempts convention to CLAUDE.md

## [3.4.62] — 2026-05-03 22:29 ET

### Changes
- chore: replace issue template with typed feature/bug/idea templates + depends_on front matter
- feat(palette): Cmd+J / Cmd+K as ArrowDown / ArrowUp aliases in command palette (#620)

## [3.4.61] — 2026-05-03 22:20 ET

### Changes

## [3.4.60] — 2026-05-03 22:07 ET

### Changes

## [3.4.59] — 2026-05-03 21:34 ET

### Changes

## [3.4.58] — 2026-05-03 21:27 ET

### Changes
- fix(config): confirm_close defaults to false, matching template and UX intent (#614)

## [3.4.57] — 2026-05-03 21:24 ET

### Changes
- refactor(pane): kill Pane::Agent and Pane::AgentWorkspace (#523) (#612)

## [3.4.56] — 2026-05-03 20:36 ET

### Changes
- feat(cli): plexi update — detached relaunch when run from inside Plexi (#606)

## [3.4.55] — 2026-05-03 19:46 ET

### Changes
- docs: scrollback navigation + copy-mode design spec (#602, #603)
- feat(cli): implement plexi update — binary self-update for stable channel (#594) (#601)

## [3.4.54] — 2026-05-03 19:18 ET

### Changes
- docs: DEV_LOG PR #600 — plexi update namespace rename
- refactor(cli): reserve 'plexi update' for self-update; move app updates to 'plexi update apps' (#600)

## [3.4.53] — 2026-05-03 18:57 ET

### Changes
- docs: DEV_LOG PR #598 — SpawnPane, plexi open CLI, SDK ctx.spawn_pane()

## [3.4.52] — 2026-05-03 18:23 ET

### Changes
- docs: DEV_LOG PR #596 — empty-state notification modal
- docs: SpawnPane design spec — overlay layout types, plexi open CLI, pipe-back handoff
- fix(notifications): Cmd+Shift+A always opens modal, empty state when queue is empty (#596)

## [3.4.51] — 2026-05-03 18:08 ET

### Changes
- docs: DEV_LOG PR #586 — notify choice response file fix
- docs(lessons): command self-containment — handler data belongs in the command, not ambient state
- feat(cli): plexi notify --choice blocking flow (#586)

## [3.4.50] — 2026-05-03 17:47 ET

### Changes
- docs: DEV_LOG PR #589 — defer heartbeat until after shell probes
- chore: replace mit label with P0 (won't fix) in priority scale
- fix(startup): defer heartbeat until after shell probes (#588) (#589)
- docs: require instrumentation for new features; document macOS file drag pointer state gotcha

## [3.4.49] — 2026-05-03 16:25 ET

### Changes
- docs: DEV_LOG PR #585 — freeze + drop fix on zoomed pane
- fix(promote): correct changelog range + clean up corrupt 3.4.48 entry
- fix(drag): skip TerminalView render during file hover on zoomed pane (#585)
- chore: remove redundant skill callouts from CLAUDE.md; add triage YAML frontmatter
- chore: promote to beta — v3.4.48

## [3.4.48] — 2026-05-03

### Changes
- chore: remove redundant skill callouts from CLAUDE.md; add triage YAML frontmatter

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

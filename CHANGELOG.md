# Changelog

Newest releases appear first.
## [0.0.411] — 2026-05-16

### Changes
- fix(keys): change scratchpad trigger from Ctrl+Space to Cmd+Shift+Space (#1380) (#1407)
- feat(quick-note): add 'n' shortcut to open config from destination picker (#1403)
- chore(config): gut unimplemented voice config infrastructure (#1401)
- fix(audio): eliminate save hang and improve pipe teardown (#1389) (#1400)
- feat(cli): add 'plexi config edit' command (#1387) (#1399)
- docs(config): add catppuccin-latte and solarized-light to preset header comment (#1396)
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.410] — 2026-05-16

### Changes
- feat(quick-note): add 'n' shortcut to open config from destination picker (#1403)
- chore(config): gut unimplemented voice config infrastructure (#1401)
- fix(audio): eliminate save hang and improve pipe teardown (#1389) (#1400)
- feat(cli): add 'plexi config edit' command (#1387) (#1399)
- docs(config): add catppuccin-latte and solarized-light to preset header comment (#1396)
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.409] — 2026-05-16

### Changes
- chore(config): gut unimplemented voice config infrastructure (#1401)
- fix(audio): eliminate save hang and improve pipe teardown (#1389) (#1400)
- feat(cli): add 'plexi config edit' command (#1387) (#1399)
- docs(config): add catppuccin-latte and solarized-light to preset header comment (#1396)
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.408] — 2026-05-16

### Changes
- fix(audio): eliminate save hang and improve pipe teardown (#1389) (#1400)
- feat(cli): add 'plexi config edit' command (#1387) (#1399)
- docs(config): add catppuccin-latte and solarized-light to preset header comment (#1396)
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.407] — 2026-05-16

### Changes
- feat(cli): add 'plexi config edit' command (#1387) (#1399)
- docs(config): add catppuccin-latte and solarized-light to preset header comment (#1396)
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.406] — 2026-05-16

### Changes
- docs(config): add catppuccin-latte and solarized-light to preset header comment (#1396)
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.405] — 2026-05-16

### Changes
- Enable osc_pane_title by default (#1395)
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.404] — 2026-05-16

### Changes
- feat(theme): add catppuccin-latte and solarized-light presets (#1386) (#1394)
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.403] — 2026-05-16

### Changes
- fix(examples/screen-time): shift clock ring up in narrow mode to clear legend (#1393)
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.402] — 2026-05-16

### Changes
- feat(examples): pixel art tavern — NPC conversation via ai_query + PGAP canvas (#1361) (#1363)
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.401] — 2026-05-16

### Changes
- feat(contexts): fractal sub-contexts with spatial zoom-in/zoom-out (#1374) (#1377)
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.400] — 2026-05-16

### Changes
- perf(deps): remove dead hound dep, upgrade rodio/rfd/objc2 to eliminate cpal duplicate (#1309) (#1376)
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.399] — 2026-05-16

### Changes
- refactor(config): consolidate CONFIG_TEMPLATE into single source of truth (#1121) (#1375)
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.398] — 2026-05-16

### Changes
- fix(website): remove MIT wording, sync version, fix blog date, first-person voice (#1372) (#1373)
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.397] — 2026-05-16

### Changes
- feat(cli): channel-aware shell completions (#1316) (#1371)
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.396] — 2026-05-16

### Changes
- perf(ai): async generation metrics fetch — eliminate 7s post-stream block (#1352) (#1370)
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.395] — 2026-05-16

### Changes
- refactor(inspector): extract draw_context_inspector into helpers — 358 lines → 120 (#1368)
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.394] — 2026-05-16

### Changes
- fix(infra): seed PR build config from alpha instead of blank template (#1369)
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.393] — 2026-05-16

### Changes
- fix(terminal): first pane from welcome screen falls back to context path instead of / (#1351) (#1364)
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.393] — 2026-05-16

### Changes
- feat(host): capability pre-flight check — error tile when app lacks required config (#1345) (#1366)
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.392] — 2026-05-16

### Changes
- fix(infra): atomic SDK swap in install.sh to prevent TOCTOU crash (#1324) (#1365)
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.391] — 2026-05-16

### Changes
- feat(sdk/host): async image loading — emit.load_image() handle/lifecycle (#1354) (#1362)
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.390] — 2026-05-16

### Changes
- feat(ui): configurable unfocused-pane opacity via ghost_opacity (#1350) (#1359)
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.389] — 2026-05-16

### Changes
- fix(app-init): pass workspace cwd to host so auto-open finds the new app (#1360)
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.388] — 2026-05-16

### Changes
- feat(host/sdk): static capability validation at app launch (#1355) (#1357)
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.387] — 2026-05-16

### Changes
- fix(host): fetch https:// URLs in DrawCommand::Image via net.http (#1353) (#1356)
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.386] — 2026-05-16

### Changes
- feat(examples): n8n-style node canvas POC — interactive graph editor in PGAP (#1338) (#1349)
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.385] — 2026-05-16

### Changes
- fix(ai): increase OpenRouter generation metrics retry window for Gemini (#1339) (#1348)
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.384] — 2026-05-16

### Changes
- feat(sdk): add emit.schedule_task() and guard run_sync() against deadlock (#1340) (#1341)
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.383] — 2026-05-16

### Changes
- fix(ai): broadcast fresh AI config to all panes on reload (#1337) (#1342)
- feat(cli): plexi uninstall for Plexi self-removal, app uninstall cleanup (#1333) (#1335)
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.382] — 2026-05-15

### Changes
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.381] — 2026-05-15

### Changes
- docs(cli): rewrite all help text for vibe coder clarity (#1322) (#1334)
- fix(install): improve installer tone and soften admin access warning
## [0.0.380] — 2026-05-15

### Changes
- fix(install): use North Star tagline, fix read prompt in curl|sh pipe
## [0.0.379] — 2026-05-15

### Changes
- fix(install): align banner boxes dynamically, add Enter confirmation, fix bullet text
## [0.0.378] — 2026-05-15

### Changes
- fix(install): dynamic banner alignment, replace broken shell-init with completions, clean dead PR symlinks
## [0.0.377] — 2026-05-15

### Changes
- chore: reset changelog for v0.x era
- chore: reset version to 0.0.376, revamp install script
- ui(overlays): move coffee link below email, shrink to caption, rename label (#1329) (#1331)
- feat(welcome): add early-stage disclaimer to welcome screen
- fix(inspector): cap pane list height with ScrollArea to prevent viewport overflow (#1327)
- docs: add CLI-as-product design principle to North Star
- docs: add target audience and design principle to North Star
- feat(panes): inspector shows all contexts + focus; fix frame flicker on reload (#1297, #1298) (#1317)
- chore(skills): move plexi skills to tracked skills/ dir
- chore(skills): restore plexi-cli, create-plexi-app, plexi-install as .agents symlinks
- ci: add workflow_dispatch trigger to check workflows
- fix(ci): switch check workflows to macos-latest
- chore: regenerate PGAP schema to match current Rust types
- refactor(protocol): rename HostCommand → AppRequest (#1311) (#1314)
- feat(sdk): expose CLI arguments as self.args on App class (#1315)
- fix(scratchpad): replace double-space trigger with Ctrl+Space (#1310) (#1313)
- feat(website): add narrative story sections to landing page
- feat(website): refresh hero copy, add mobile nav, move support to own page

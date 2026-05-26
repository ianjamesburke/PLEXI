# Changelog

Newest releases appear first.
## [0.0.508] — 2026-05-26

### Changes
- feat(cli): workspace app manifest — .plexi/apps.toml declares per-project app dependencies (#1719) (#1750)
- chore: restore session edits carried through merge
- Bug: Pane focus misbehaves on close and fullscreen toggle (#1547) (#1753)
- feat(cli): plexi demo — live keybinding tutorial that watches events.jsonl (#1740) (#1749)
- chore: remove demo spec file
- feat(website): swap mailing list storage from SQLite to Resend Contacts
- docs: add plexi demo interactive tutorial spec
- chore(justfile): auto-commit and push CLI docs from gen-cli-docs recipe
- chore(website): remove deprecated link and unlink command documentation
## [0.0.507] — 2026-05-26

### Changes
- chore: restore session edits carried through merge
- Bug: Pane focus misbehaves on close and fullscreen toggle (#1547) (#1753)
- feat(cli): plexi demo — live keybinding tutorial that watches events.jsonl (#1740) (#1749)
- chore: remove demo spec file
- feat(website): swap mailing list storage from SQLite to Resend Contacts
- docs: add plexi demo interactive tutorial spec
- chore(justfile): auto-commit and push CLI docs from gen-cli-docs recipe
- chore(website): remove deprecated link and unlink command documentation
## [0.0.506] — 2026-05-26

### Changes
- feat(cli): plexi demo — live keybinding tutorial that watches events.jsonl (#1740) (#1749)
- chore: remove demo spec file
- feat(website): swap mailing list storage from SQLite to Resend Contacts
- docs: add plexi demo interactive tutorial spec
- chore(justfile): auto-commit and push CLI docs from gen-cli-docs recipe
- chore(website): remove deprecated link and unlink command documentation
## [0.0.505] — 2026-05-26

### Changes
- chore(cli): remove deprecated app link / app unlink commands and links.toml infrastructure (#1704) (#1730)
- chore: update pgap schema + validate-pr skill for ListView types
- feat(sdk/pgap): List + Row — host-native scrollable list container with generic slotted row (#1727) (#1728)
- chore: update generated schema and docs for --from-cursor flag (v0.0.503)
- feat(cli/terminal): cursor-based incremental pane capture (#1723) (#1725)
- docs(skills/implement-issue): alpha gate is first check in pre-flight, before any gh calls
- docs(skills): unify ship pipeline to inline chaining, remove PM-dispatch model
- fix(apps/gh-issues): label badges overlap title text (#1724) (#1726)
- docs(skills/plexi-cli): correct pane send newline behavior
- refactor(apps): flatten app directory — collapse core/examples into apps/, move dev-examples to apps/dev/ (#1718) (#1721)
- fix(cli): workspace init does not create channel dir — resolve_workspace_root fails on non-stable channels (#1720) (#1722)
- fix(cli): pane-spawning commands split from PLEXI_PANE_ID env, not UI-active pane
- refactor(apps): overhaul app directory structure — core/fixtures split + registry-driven distribution (#1295) (#1717)
- feat(infra/skills): PM as single IO channel — pipeline orchestration via GitHub labels (#1670)
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.504] — 2026-05-26

### Changes
- feat(sdk/pgap): List + Row — host-native scrollable list container with generic slotted row (#1727) (#1728)
- chore: update generated schema and docs for --from-cursor flag (v0.0.503)
- feat(cli/terminal): cursor-based incremental pane capture (#1723) (#1725)
- docs(skills/implement-issue): alpha gate is first check in pre-flight, before any gh calls
- docs(skills): unify ship pipeline to inline chaining, remove PM-dispatch model
- fix(apps/gh-issues): label badges overlap title text (#1724) (#1726)
- docs(skills/plexi-cli): correct pane send newline behavior
- refactor(apps): flatten app directory — collapse core/examples into apps/, move dev-examples to apps/dev/ (#1718) (#1721)
- fix(cli): workspace init does not create channel dir — resolve_workspace_root fails on non-stable channels (#1720) (#1722)
- fix(cli): pane-spawning commands split from PLEXI_PANE_ID env, not UI-active pane
- refactor(apps): overhaul app directory structure — core/fixtures split + registry-driven distribution (#1295) (#1717)
- feat(infra/skills): PM as single IO channel — pipeline orchestration via GitHub labels (#1670)
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.503] — 2026-05-26

### Changes
- feat(cli/terminal): cursor-based incremental pane capture (#1723) (#1725)
- docs(skills/implement-issue): alpha gate is first check in pre-flight, before any gh calls
- docs(skills): unify ship pipeline to inline chaining, remove PM-dispatch model
- fix(apps/gh-issues): label badges overlap title text (#1724) (#1726)
- docs(skills/plexi-cli): correct pane send newline behavior
- refactor(apps): flatten app directory — collapse core/examples into apps/, move dev-examples to apps/dev/ (#1718) (#1721)
- fix(cli): workspace init does not create channel dir — resolve_workspace_root fails on non-stable channels (#1720) (#1722)
- fix(cli): pane-spawning commands split from PLEXI_PANE_ID env, not UI-active pane
- refactor(apps): overhaul app directory structure — core/fixtures split + registry-driven distribution (#1295) (#1717)
- feat(infra/skills): PM as single IO channel — pipeline orchestration via GitHub labels (#1670)
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.502] — 2026-05-25

### Changes
- fix(apps/gh-issues): label badges overlap title text (#1724) (#1726)
- docs(skills/plexi-cli): correct pane send newline behavior
- refactor(apps): flatten app directory — collapse core/examples into apps/, move dev-examples to apps/dev/ (#1718) (#1721)
- fix(cli): workspace init does not create channel dir — resolve_workspace_root fails on non-stable channels (#1720) (#1722)
- fix(cli): pane-spawning commands split from PLEXI_PANE_ID env, not UI-active pane
- refactor(apps): overhaul app directory structure — core/fixtures split + registry-driven distribution (#1295) (#1717)
- feat(infra/skills): PM as single IO channel — pipeline orchestration via GitHub labels (#1670)
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.501] — 2026-05-25

### Changes
- refactor(apps): flatten app directory — collapse core/examples into apps/, move dev-examples to apps/dev/ (#1718) (#1721)
- fix(cli): workspace init does not create channel dir — resolve_workspace_root fails on non-stable channels (#1720) (#1722)
- fix(cli): pane-spawning commands split from PLEXI_PANE_ID env, not UI-active pane
- refactor(apps): overhaul app directory structure — core/fixtures split + registry-driven distribution (#1295) (#1717)
- feat(infra/skills): PM as single IO channel — pipeline orchestration via GitHub labels (#1670)
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.500] — 2026-05-24

### Changes
- refactor(apps): overhaul app directory structure — core/fixtures split + registry-driven distribution (#1295) (#1717)
- feat(infra/skills): PM as single IO channel — pipeline orchestration via GitHub labels (#1670)
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.499] — 2026-05-24

### Changes
- refactor(infra): unified channel primitive with stable→main rename (#1711) (#1714)
- bug(host): command palette app list is stale — registry not re-scanned after app init or filesystem changes (#1712) (#1716)
- fix(website): add missing language tags to bare code fences
- fix(cli): app init workspace detection checks .plexi/ not channel dir
- fix(website): rewrite sdk.md with proper markdown formatting
- feat(sdk): scaffold template demonstrates state persistence with hot reload
- fix(website): remove unpublished SDK install instructions, bump verified version
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.498] — 2026-05-24

### Changes
- fix(host/pane-ops): honor from_pane_id for app spawns (#1705) (#1708)
- refactor(skills): consolidate ship pipeline — delete ship-issue, centralize env, move hand-off (#1709)
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.497] — 2026-05-24

### Changes
- fix(host): hot reload never fires for apps launched via path (app init / app run) (#1706) (#1707)
## [0.0.496] — 2026-05-23

### Changes
- fix(sdk): ctx.copy_to_clipboard writes immediately via _emit, not _queue (#1697)
- fix(skills/dispatch): open-lanes.sh dispatches implement-issue directly (#1696)
- refactor(skills): collapse 4-phase ship pipeline to 2-phase with hand-off (#1695)
- fix(skills): reduce implement-issue pre-flight to ≤3 tool calls when dispatched with specific number (#1673)
- chore(dispatch): rewrite skill to spawn Claude sub-agents instead of Plexi panes
- chore: update dispatch to use /ship-issue and clarify validate-pr to merge-pr handoff
- chore: move merge-pr completion output to close sequence
- chore: generated files from v0.0.494 build
- chore: restore session edits carried through merge
- feat(cli): plexi pane list --context <id> to filter panes by context (#1664) (#1681)
- docs(cli): update verified version to 0.0.493
- revert: P0 kill + bare-args guard — two regressions, back to drawing board
- fix(cli): gui launch from Finder/Dock broken by bare-args exit path (#1678)
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.495] — 2026-05-23

### Changes
- fix(skills): reduce implement-issue pre-flight to ≤3 tool calls when dispatched with specific number (#1673)
- chore(dispatch): rewrite skill to spawn Claude sub-agents instead of Plexi panes
- chore: update dispatch to use /ship-issue and clarify validate-pr to merge-pr handoff
- chore: move merge-pr completion output to close sequence
- chore: generated files from v0.0.494 build
- chore: restore session edits carried through merge
- feat(cli): plexi pane list --context <id> to filter panes by context (#1664) (#1681)
- docs(cli): update verified version to 0.0.493
- revert: P0 kill + bare-args guard — two regressions, back to drawing board
- fix(cli): gui launch from Finder/Dock broken by bare-args exit path (#1678)
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.494] — 2026-05-23

### Changes
- chore: restore session edits carried through merge
- feat(cli): plexi pane list --context <id> to filter panes by context (#1664) (#1681)
- docs(cli): update verified version to 0.0.493
- revert: P0 kill + bare-args guard — two regressions, back to drawing board
- fix(cli): gui launch from Finder/Dock broken by bare-args exit path (#1678)
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.493] — 2026-05-23

### Changes
- revert: P0 kill + bare-args guard — two regressions, back to drawing board
- fix(cli): gui launch from Finder/Dock broken by bare-args exit path (#1678)
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.492] — 2026-05-23

### Changes
- fix(cli): gui launch from Finder/Dock broken by bare-args exit path (#1678)
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.491] — 2026-05-22

### Changes
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.490] — 2026-05-22

### Changes
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.489] — 2026-05-22

### Changes
- feat(host/terminal): Cmd+A full-pane selection + cross-pane selection persistence (#1629) (#1677)
- fix(overlays): context inspector UI audit — OSC status, close button, title, set-root-path button (#1627) (#1675)
- fix(apps/logs): copy mode — mouse selection and shift-click multi-line copy (#1632) (#1676)
- P0: `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` (#1546) (#1674)
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.488] — 2026-05-22

### Changes
- P0: `plexi open config.toml` does nothing on machines without VS Code (#1545) (#1667)
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.487] — 2026-05-22

### Changes
- fix(apps/bluesky): Esc key does not close the Bluesky app (#1654) (#1663)
- chore(skills): simplify dispatch gate, skip review for bundle PRs
- chore(skills): pass origin/alpha base to wtp add; add V1_ROADMAP
- fix(dispatch): simplify open-lanes.sh — use plexi terminal cmd directly, tighten dirty check to tracked files only
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.486] — 2026-05-22

### Changes
- fix(ui/sidebar): show dot when context has exactly one pane (#1662)
- docs(skill): update create-plexi-app dev loop and scaffold guidance
- chore: gitignore channel profile dirs and agent memory
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.485] — 2026-05-22

### Changes
- fix(host/terminal): QuickNote paste falls through to terminal behind (#1637)
- docs: log QuickNote paste-through fix in DEV_LOG
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.484] — 2026-05-22

### Changes
- fix(cli): print launch notice when plexi is run with no arguments (#1515) (#1657)
- docs: log NSWindow appearance fix for black title in light mode
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.483] — 2026-05-22

### Changes
- fix(host): sync NSWindow appearance with Plexi theme to fix black title in light mode (#1656)
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.482] — 2026-05-22

### Changes
- fix(host): use retain in all sync_*_focus methods to remove stale buried layers (#1598) (#1655)
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.481] — 2026-05-22

### Changes
- feat(sdk): feed-quality primitives — max_lines, avatar, skeleton, measure_text_wrapped (#1607) (#1610)
- feat(infra/agents): pick-parallel skill for selecting parallelizable issues
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.480] — 2026-05-22

### Changes
- feat(infra/agents): project-manager skill — conviction-scored parallel dispatch (#1618) (#1619)
- refactor(host): explicit pane targeting — eliminate focused_pane save/restore in IPC handlers (#1613) (#1620)
- docs: log CapabilityModal FocusLayer fix and egui_tiles bare-pane-root gotcha
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.479] — 2026-05-22

### Changes
- fix(host): promote CapabilityModal to FocusLayer for exclusive keyboard input (#1596)
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.478] — 2026-05-21

### Changes
- feat(cli): plexi open github:owner/repo — ephemeral launch without install (#1530) (#1617)
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.477] — 2026-05-21

### Changes
- fix(icon): replace stale terminal-grid PNG with current pane-layout icon (#1615)
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.476] — 2026-05-21

### Changes
- fix(cli): app init no-workspace fallback uses global profile dir instead of CWD (#1614)
- improve(plexi-cli): sync skill to live binary v0.0.475
- improve(plexi-cli): replace deprecated app link with app run <path>
- improve(plexi-cli): add app render ID requirement and --help-first rule
- fix(merge-pr): handle remote branch already deleted during cleanup
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.475] — 2026-05-20

### Changes
- feat(logs): search, copy mode, and row/badge spacing polish (#1593)
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.474] — 2026-05-20

### Changes
- feat(skills): composable ship pipeline — implement-issue, open-pr, validate-pr, merge-pr, test-pr (#1602)
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.473] — 2026-05-20

### Changes
- feat(permissions): capability grant/deny modal — 4-action logic, SDK typed error, example apps (#1582)
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.472] — 2026-05-20

### Changes
- fix(backlog): channel-aware quick-note backlog (#1595) (#1597)
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.471] — 2026-05-20

### Changes
- feat(host): emit SESSION START/END log markers on process app lifecycle (#1511) (#1589)
- chore(skill): bump create-plexi-app skill_version to 0.0.470 (#1510)
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.470] — 2026-05-20

### Changes
- fix(cli): plexi open exits 0 with pane ID when app launch silently fails (#1590)
- fix: bundle — inspector Cmd+R rename, skill version, registry rename (#1508, #1510, #1537) (#1588)
- docs(dispatch): add lane recovery rules and mandatory stabilizer note
- fix(dispatch): add dirty-tree guard and shell-ready delays to open-lanes.sh
- docs(website): update SDK reference pages
- feat(website): replace hero placeholder with real screenshot
## [0.0.469] — 2026-05-19

### Changes
- feat(sdk): default PGAP theme — auto-clear BG before on_render (#1576) (#1586)
- feat(cli): first-launch completions detection and banner (#1577) (#1585)
- fix(skills): include PR link in install-needed testing block
- fix(apps): align Calculator keyboard shortcuts with SDK conventions (#1573) (#1583)
- fix(apps): request audio.record capability on init in audio-recorder (#1584)
- fix(skills): fail-fast rule for ship-issue, dev log updates
- fix(skills): fail-fast on unrecoverable errors, skip review in parallel dispatch
- fix(apps): fix Wikipedia key names, Bluesky JSON errors, and net.http capability consent (#1567, #1569)
- docs(cli): update uninstall --yes description and verified_version
- fix(dispatch): prompt user about dirty alpha instead of hard-stopping
- fix(dispatch): cd into repo root before sending ship command
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.468] — 2026-05-19

### Changes
- feat(sdk): default PGAP theme — auto-clear BG before on_render (#1576) (#1586)
- feat(cli): first-launch completions detection and banner (#1577) (#1585)
- fix(skills): include PR link in install-needed testing block
- fix(apps): align Calculator keyboard shortcuts with SDK conventions (#1573) (#1583)
- fix(apps): request audio.record capability on init in audio-recorder (#1584)
- fix(skills): fail-fast rule for ship-issue, dev log updates
- fix(skills): fail-fast on unrecoverable errors, skip review in parallel dispatch
- fix(apps): fix Wikipedia key names, Bluesky JSON errors, and net.http capability consent (#1567, #1569)
- docs(cli): update uninstall --yes description and verified_version
- fix(dispatch): prompt user about dirty alpha instead of hard-stopping
- fix(dispatch): cd into repo root before sending ship command
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.467] — 2026-05-19

### Changes
- feat(cli): first-launch completions detection and banner (#1577) (#1585)
- fix(skills): include PR link in install-needed testing block
- fix(apps): align Calculator keyboard shortcuts with SDK conventions (#1573) (#1583)
- fix(apps): request audio.record capability on init in audio-recorder (#1584)
- fix(skills): fail-fast rule for ship-issue, dev log updates
- fix(skills): fail-fast on unrecoverable errors, skip review in parallel dispatch
- fix(apps): fix Wikipedia key names, Bluesky JSON errors, and net.http capability consent (#1567, #1569)
- docs(cli): update uninstall --yes description and verified_version
- fix(dispatch): prompt user about dirty alpha instead of hard-stopping
- fix(dispatch): cd into repo root before sending ship command
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.466] — 2026-05-19

### Changes
- fix(apps): align Calculator keyboard shortcuts with SDK conventions (#1573) (#1583)
- fix(apps): request audio.record capability on init in audio-recorder (#1584)
- fix(skills): fail-fast rule for ship-issue, dev log updates
- fix(skills): fail-fast on unrecoverable errors, skip review in parallel dispatch
- fix(apps): fix Wikipedia key names, Bluesky JSON errors, and net.http capability consent (#1567, #1569)
- docs(cli): update uninstall --yes description and verified_version
- fix(dispatch): prompt user about dirty alpha instead of hard-stopping
- fix(dispatch): cd into repo root before sending ship command
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.465] — 2026-05-19

### Changes
- fix(apps): request audio.record capability on init in audio-recorder (#1584)
- fix(skills): fail-fast rule for ship-issue, dev log updates
- fix(skills): fail-fast on unrecoverable errors, skip review in parallel dispatch
- fix(apps): fix Wikipedia key names, Bluesky JSON errors, and net.http capability consent (#1567, #1569)
- docs(cli): update uninstall --yes description and verified_version
- fix(dispatch): prompt user about dirty alpha instead of hard-stopping
- fix(dispatch): cd into repo root before sending ship command
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.464] — 2026-05-19

### Changes
- fix(apps): fix Wikipedia key names, Bluesky JSON errors, and net.http capability consent (#1567, #1569)
- docs(cli): update uninstall --yes description and verified_version
- fix(dispatch): prompt user about dirty alpha instead of hard-stopping
- fix(dispatch): cd into repo root before sending ship command
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.463] — 2026-05-19

### Changes
- feat(cli): simplify uninstall to single confirmation prompt (#1574) (#1578)
- fix(cli): show help inside pane, hint on missing script (#1570, #1571) (#1580)
- chore(skills): bump create-plexi-app skill_version to 0.0.461
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.462] — 2026-05-19

### Changes
- fix(apps): replace misleading empty-state message in Backlog app (#1572) (#1579)
- fix(inspector): fix non-Process app status, drop raw delete Button from footer (#1565)
- fix(file-browser): open at ~ instead of / when launched as GUI app (#1561) (#1562)
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.461] — 2026-05-19

### Changes
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.460] — 2026-05-19

### Changes
- chore(skills): add create-plexi-app gate to ship-issue Phase 3 (#1509) (#1564)
- chore: throttle cargo build jobs to 3/10 cores for CPU headroom
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.459] — 2026-05-19

### Changes
- feat(widgets): overlay layout primitives — section_header, pane_type_badge, status_chip, description_label (#1540) (#1541)
- fix(ship-skill): block Phase 1 rebase if alpha has unpushed commits
- chore(skills): rename dispatch-next → dispatch, add scripts, fix layout + channel handling
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.458] — 2026-05-19

### Changes
- fix(pane-ops): Cmd+N from welcome screen now opens at context root (#1534) (#1536)
- chore: restore dispatch pattern in CLAUDE.md (reverted by external edit)
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.457] — 2026-05-19

### Changes
- docs: write CLI descriptor authoring guide (--plexi flag) (#1528) (#1535)
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.456] — 2026-05-19

### Changes
- fix(terminal): always use TERM=xterm-256color, drop xterm-ghostty (#1533)
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.455] — 2026-05-19

### Changes
- feat(context): context tree rework — unified types, portals, agent spawning, anchors (#1516, #1517, #1518, #1520) (#1525)
- fix(cli): app init walks up to nearest channel-aware workspace dir (#1526)
## [0.0.454] — 2026-05-18

### Changes
## [0.0.453] — 2026-05-18

### Changes
- fix(infra): override hardcoded target-dir in release workflow (#1522) (#1523)
- fix(cli): app init scaffolds into <cwd>/.plexi/apps/<name>/ (#1519) (#1521)
## [0.0.452] — 2026-05-18

### Changes
- fix(cli): app init scaffolds into <cwd>/.plexi/apps/<name>/ (#1519) (#1521)
## [0.0.451] — 2026-05-18

### Changes
- fix(file-browser): letter keys append to search query; Escape exits search (#1512)
- feat: add audio device protocol types and dispatch-agents skill
- feat(inspector): workspace HUD — neutral header, active context accent (#1326) (#1488)
- feat(inspector): inline context name editing from inspector modal (#1289) (#1486)
- fix(app): plexi-logs — ctx.button() for filter chips, ctx.badge() for level labels (#1504)
- chore: regenerate schema, Python SDK protocol, and CLI docs for v0.0.449
- feat(skills): add --skip-review flag to ship-issue skill
- feat(context): add optional description field with sidebar, inspector, CLI, env var, and quick note support (#1120) (#1506)
- feat(apps): app startup message written to companion terminal on launch (#185) (#1498)
- fix(overlays): prevent terminal panes from stealing egui focus while a modal is open (#1501)
- feat(sdk+manifest): app size classes — min size guard + compact/regular/full breakpoints (#423) (#1502)
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.450] — 2026-05-18

### Changes
- feat(inspector): workspace HUD — neutral header, active context accent (#1326) (#1488)
- feat(inspector): inline context name editing from inspector modal (#1289) (#1486)
- fix(app): plexi-logs — ctx.button() for filter chips, ctx.badge() for level labels (#1504)
- chore: regenerate schema, Python SDK protocol, and CLI docs for v0.0.449
- feat(skills): add --skip-review flag to ship-issue skill
- feat(context): add optional description field with sidebar, inspector, CLI, env var, and quick note support (#1120) (#1506)
- feat(apps): app startup message written to companion terminal on launch (#185) (#1498)
- fix(overlays): prevent terminal panes from stealing egui focus while a modal is open (#1501)
- feat(sdk+manifest): app size classes — min size guard + compact/regular/full breakpoints (#423) (#1502)
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.449] — 2026-05-18

### Changes
- feat(context): add optional description field with sidebar, inspector, CLI, env var, and quick note support (#1120) (#1506)
- feat(apps): app startup message written to companion terminal on launch (#185) (#1498)
- fix(overlays): prevent terminal panes from stealing egui focus while a modal is open (#1501)
- feat(sdk+manifest): app size classes — min size guard + compact/regular/full breakpoints (#423) (#1502)
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.448] — 2026-05-18

### Changes
- feat(apps): app startup message written to companion terminal on launch (#185) (#1498)
- fix(overlays): prevent terminal panes from stealing egui focus while a modal is open (#1501)
- feat(sdk+manifest): app size classes — min size guard + compact/regular/full breakpoints (#423) (#1502)
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.447] — 2026-05-18

### Changes
- fix(overlays): prevent terminal panes from stealing egui focus while a modal is open (#1501)
- feat(sdk+manifest): app size classes — min size guard + compact/regular/full breakpoints (#423) (#1502)
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.446] — 2026-05-18

### Changes
- feat(sdk+manifest): app size classes — min size guard + compact/regular/full breakpoints (#423) (#1502)
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.445] — 2026-05-18

### Changes
- feat(contexts): portal sub-contexts — kill adoption, cascade delete, unlimited nesting (#1392) (#1503)
- fix(ship-skill): require full path for PR binary in test scripts
- docs: regenerate CLI docs for v0.0.444
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.444] — 2026-05-18

### Changes
- feat(scratchpad): timestamped per-session notes and CLI browser (#1419) (#1499)
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.443] — 2026-05-18

### Changes
- feat(sdk): non-blocking notify_*_async wrappers (#310) (#1496)
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.442] — 2026-05-18

### Changes
- docs(skills): add app layout safety rules to create-plexi-app (#1489) (#1492)
- docs(sdk): add keyboard conventions and SelectList guidance to SDK docs (#242) (#1493)
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.441] — 2026-05-18

### Changes
- feat(contexts): close-context dialog with pane list and dissolve option (#1385) (#1474)
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.440] — 2026-05-18

### Changes
- fix(navigation): boundary jump up focuses leftmost pane in destination window (#1479)
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.439] — 2026-05-18

### Changes
- fix(inspector): remove ✕ close button from rows, add ⌘W shortcut (#1475)
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.438] — 2026-05-18

### Changes
- feat(cli): add tips system with config toggle (#1323) (#1487)
- fix(build): add missing new_pane_first arg to split_focused call
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.437] — 2026-05-18

### Changes
- feat(notify): persist pending notifications to disk across restarts (#705) (#1484)
- feat(scheduler): workspace-scoped routines — shell commands on schedule (#1098) (#1481)
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.436] — 2026-05-18

### Changes
- feat(cli): pane capture strips trailing empty lines by default, add --full-output flag (#1483)
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.435] — 2026-05-18

### Changes
- ux(notifications): replace ⌘[ / ⌘] cycle hint with H / L in modal header (#1424) (#1480)
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.434] — 2026-05-18

### Changes
- feat(layout): add split_left — new pane opens left of focused pane (#1430) (#1476)
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.433] — 2026-05-18

### Changes
- docs(readme): add contact section before install (#478) (#1478)
- docs(gotchas): proc_listchildpids(NULL,0) returns EFAULT on macOS 23.x
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.432] — 2026-05-18

### Changes
- feat(inspector): smarter pane status — idle vs busy shell, OSC title awareness (#1288) (#1472)
- refactor(widgets): extract styled_text_input() helper — deduplicates modal text input pattern (#1471)
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.431] — 2026-05-18

### Changes
- fix(overlays): context inspector UI audit — centering, title, terminal detail (#1468)
- feat(permissions): first-run consent prompt for sensitive capabilities (#1455) (#1467)
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.430] — 2026-05-18

### Changes
- feat(install): print dotfiles hint after successful install (#1469)
- fix(keys): swap Cmd+D / Cmd+Shift+D split polarity (#1466)
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.429] — 2026-05-18

### Changes
- fix(apps): pixel tavern — persistent bubbles, parchment styling, terse system prompts (#1379) (#1461)
- feat(context): sub-context creation auto-zooms in, ChildPaneSummary for tile previews (#1409) (#1464)
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.428] — 2026-05-18

### Changes
- fix(overlays): context inspector pre-selects focused pane on open (#1434) (#1462)
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.427] — 2026-05-18

### Changes
- feat(config): [agents] block — tiered coding agent command templates (#1397) (#1459)
- feat(cli): add `plexi app run <path>` and deprecate `app link`/`app unlink` (#1408) (#1460)
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.426] — 2026-05-18

### Changes
- fix(ai): block on background metrics so AiResponse carries accurate token counts (#1458)
- fix(overlays): Cmd+0 quick note works when another modal is open (#1435) (#1457)
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.425] — 2026-05-18

### Changes
- refactor(sidebar): replace SidebarRow pixel math with ContextItem scope+shape layout (#1448) (#1454)
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.424] — 2026-05-18

### Changes
- ux(install): replace 'restart your terminal' with 'close this terminal and open Plexi' (#1437) (#1452)
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.423] — 2026-05-18

### Changes
- chore(examples): flatten examples/apps/ into examples/ root (#1450)
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.422] — 2026-05-18

### Changes
- feat(install): bundle and install skills in user-install.sh (#1451)
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.421] — 2026-05-18

### Changes
- fix(cli): unify plexi list / plexi app list and fix command discovery (#1440) (#1443)
- Revert "feat(sidebar): unified context row with subtitle and pane dots (#1442)"
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.420] — 2026-05-18

### Changes
- feat(sidebar): unified context row with subtitle and pane dots (#1442)
- fix(install): add 'Check for success' button to CLI setup modal (#1439) (#1441)
- feat(overlays): TextInputOverlay primitive + context root management (#1426) (#1433)
- feat(website): surface install script as primary download CTA (#1431)
- fix(install): replace in-app CLI Install button with copyable curl one-liner (#1427) (#1432)
- feat(ui): pane dots below context names in sidebar (#1418)
- docs(north-star): rewrite audience sections, update Phase 1
- chore(github): add funding configuration
- fix(ci): resolve SDK type errors, stale CLI docs, and install pipeline gaps
- ui(welcome): move caution message above keyboard shortcuts
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.419] — 2026-05-17

### Changes
- fix(layout): align split_h/split_v naming with tmux convention (#1312) (#1423)
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
## [0.0.418] — 2026-05-17

### Changes
- feat(keys): H/L to cycle notifications, blocked in Choice kind (#1420) (#1421)
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
- feat(website): add Download nav button and pre-release gate page
- feat(pgap): responsive breakpoint layout primitive for SubContext and apps (#1404) (#1406)
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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
## [0.0.417] — 2026-05-17

### Changes
- perf: trim frame-time render work
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
- feat(website): add Download nav button and pre-release gate page
- feat(pgap): responsive breakpoint layout primitive for SubContext and apps (#1404) (#1406)
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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
## [0.0.416] — 2026-05-17

### Changes
- feat(scratchpad): replace egui overlay with terminal editor (#1282) (#1416)
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
- feat(website): add Download nav button and pre-release gate page
- feat(pgap): responsive breakpoint layout primitive for SubContext and apps (#1404) (#1406)
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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
## [0.0.415] — 2026-05-17

### Changes
- fix(contexts): sub-context adopts focused pane instead of starting empty (#1384) (#1414)
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
- feat(website): add Download nav button and pre-release gate page
- feat(pgap): responsive breakpoint layout primitive for SubContext and apps (#1404) (#1406)
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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
## [0.0.414] — 2026-05-17

### Changes
- fix(ui): context inspector delete via Backspace key (3×) (#1383) (#1413)
- Extract app init template to standalone SDK v2 Python file (#1272)
- fix(gh-projects): replace broken gh project CLI with raw GraphQL (#1319)
- feat(examples): Bluesky feed browser app (PGAP) (#1343) (#1347)
- feat(website): add Download nav button and pre-release gate page
- feat(pgap): responsive breakpoint layout primitive for SubContext and apps (#1404) (#1406)
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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
## [0.0.413] — 2026-05-17

### Changes
- feat(pgap): responsive breakpoint layout primitive for SubContext and apps (#1404) (#1406)
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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
## [0.0.412] — 2026-05-16

### Changes
- fix(website): apps page shows marketplace coming soon instead of POC list (#1405)
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

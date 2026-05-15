# Changelog

Newest releases appear first.
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

<!-- DEV_LOG.md — Newest entries first. Captures decisions, gotchas, and the WHY behind non-obvious choices. Read the top 100–150 lines to orient quickly. -->

## 2026-05-03 — [CHANGED] Readability pass — lightened fg variables and promoted content text

`--fg-2` #9aa3ad→#b8c0c8, `--fg-3` #6a727c→#8a9299, `--fg-4` bumped to match. Previously the mid-grays were nearly invisible against the `#07080a` background on Retina at 100% zoom. Also promoted description and meta value text from `--fg-3` to `--fg-2`/`--fg` in the apps pages — the rule is: readable content uses `--fg`/`--fg-2`, UI chrome and labels use `--fg-3`/`--fg-4`.

## 2026-05-03 — [CHANGED] Raised minimum font size floor to 12-13px site-wide

Everything below 12px caused sub-pixel rendering artifacts on Retina at 100% Chrome. 10px→12px, 11px→13px throughout. Footer copy/links were the worst offenders at 11px. Newsletter sub-copy bumped 13px→14px. Nothing below 12px anywhere now.

## 2026-05-03 — [GOTCHA] Zsh glob expansion breaks git add with bracket filenames

`git add src/pages/apps/[id].astro` fails in zsh with "no matches found" — zsh interprets `[id]` as a glob character class. Always quote the path: `git add "src/pages/apps/[id].astro"`. Applies to any Astro dynamic route file.

## 2026-05-03 — [DECISION] /apps route, not apps.plexiapp.com subdomain

Subdomain would require separate DNS config, a second Railway service or rewrite rules, and broken link equity. `/apps` in the existing Astro site is one new page with zero infrastructure cost. The registry and CLI both point to the same `raw.githubusercontent.com` URL regardless.

## 2026-05-03 — [DECISION] Registry in dedicated `plexi-registry` repo, fetched at build time

Three options considered: (1) registry inside main PLEXI binary repo, (2) hosted in plexi-webapp as a served JSON endpoint, (3) dedicated `plexi-registry` repo. Chose (3) — same pattern as Homebrew (`homebrew-core` separate from `brew`). CLI and webapp both fetch from `raw.githubusercontent.com/ianjamesburke/plexi-registry/main/registry.json`. Adding an app is a PR to one focused repo; no webapp redeploy needed.

## 2026-05-03 — [CHANGED] Registry synced to actual PLEXI examples directory

Removed 12 apps that don't exist in `examples/`: github-issues, git-log, git-blame, navigator, clipboard-stack, process-monitor, port-watcher, pulse, sandfall, seedclock, apiary, lichen, aquarium, pyflow. Added 11 real ones: tetris, todo, quick-note, mind-map, commit-graph, screen-time, stand-up-reminder, audio-player, video-player, video-editor, daw, storyboard. Added `"core": true` flag for all apps shipping with the PLEXI binary.

## 2026-05-03 — [FUTURE] `plexi screenshot <id>` CLI command — parked as v5+ idea (PLEXI#616)

All infra exists: `HeadlessRenderer` at `src/headless_renderer.rs`, `--render` CLI flag at `main.rs:616`, last committed frame in `ProcessApp.last_committed_frame`. Only missing piece is a `screenshot` match arm and pane ID lookup — purely a wiring job. Deliberately deferred; not worth the distraction pre-v5. When ready, follow up with a GitHub Action on `plexi-registry` to auto-commit screenshots for every app.

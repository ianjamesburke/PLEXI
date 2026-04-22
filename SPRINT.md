# Notification System + App Platform Sprint

Progress tracker for this work window. Each task is atomic: one worktree, one PR, one checkbox.
If session is interrupted, resume by reading this file — find the first unchecked item and continue.
File is committed after every completed task.

---

## Track A — Notification Protocol (sequential)

- [x] A1: Delete `docs/specs/app-infrastructure.md` (stale, misleads agents) — file never existed
- [x] A2: Rewrite `plexi_sdk.py` module docstring — full quick-start example, TypedDict types, every method documented
- [x] A2.5: Secrets UX — `inject` toggle in SecretsApp UI + help text + `emit.get_secret()` in SDK + inject flagged secrets into shell env via `build_env()` (#296)
- [x] A3: `DrawCommand::Notify` in `app_protocol.rs` + render stub in `process_app.rs` — already in protocol + routing
- [x] A4: `emit.notify()` + `emit.notify_and_wait()` in Python SDK — `emit.notify()` done, need `notify_and_wait()`
- [x] A5: Notification panel UI + badge in top bar + Cmd+Shift+A (#291)

## Track B — Background Apps (parallel after A3 starts)

- [x] B1: `background = true` in manifest schema
- [x] B2: Keep process alive in host when pane is closed
- [x] B3: Re-attach pane to running process on re-open (#292)

## Track C — HTTP Broker (parallel after A3 starts)

- [x] C1: `DrawCommand::HttpRequest` + `PlexiEvent::HttpResponse` in protocol — already implemented
- [x] C2: Host broker — check `net` capability + `allowed_hosts`, forward via `ureq`
- [x] C3: `llm` capability wired to secrets store + `emit.llm()` in SDK (#294)

## Track D — Draw Primitives

- [x] D1: `DrawCommand::Arc { cx, cy, r, start_angle, end_angle, fill }` for pie charts

## Track E — Proof of Concept Apps

- [ ] E1: **GitHub Tree Visualizer** — HTTP broker + `HOMEBREW_TAP_TOKEN` secret
       Beautiful repo tree: branches, recent commits, file structure
       Depends on: C2

- [ ] E2: **Screen Time** — pie chart of last 7 days macOS app usage
       Reads ~/Library/Application Support/com.apple.ScreenTimeAgent/ SQLite
       Depends on: D1

- [ ] E3: **Stand Up Reminder** — background app, notifies every 15 min, 100 random movement prompts
       Full proof of concept: background + timer + notify stack end-to-end
       Depends on: A5, B3

## Track F — Timer + CLI (after their dependencies land)

- [ ] F1: `timer` capability — `SetTimer` / `PlexiEvent::Timer` (#293) — after B3
- [ ] F2: `plexi notify` CLI for external processes (#295) — after A5

---

## Parallel execution strategy

- A1 + A2 immediately (fast, no deps)
- A3 + B1 + C1 + D1 in parallel (all independent protocol work)
- A4 after A3 | B2 after B1 | C2 after C1
- A5 after A4 | B3 after B2 | C3 after C2
- E1 after C2 | E2 after D1 | E3 after A5+B3
- F1 after B3 | F2 after A5

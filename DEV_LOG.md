# Dev Log

Newest entries first. Tracks architectural decisions, feature changes, and problem investigations.

## 2026-03-14 — Upgrade xterm to v6

Migrated from deprecated `xterm@5.3.0` to `@xterm/xterm@6.0.0`. The xterm project moved to a scoped package; v5 is no longer maintained. Updated asset paths to use the new scoped namespace for consistency with addon-fit. All tests pass, app builds successfully.

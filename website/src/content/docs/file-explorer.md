---
title: File Explorer
description: Browse your filesystem without leaving Plexi.
verified_version: "0.0.689"
order: 8
---

The file explorer is a built-in overlay that lets you browse and open files from within any Plexi context. Open it with `⌘E` from a terminal pane — no shell command required.

## Opening the File Explorer

Press `⌘E` from any focused terminal pane. The overlay opens over your current layout. It starts at the current working directory of that pane (tracked via OSC 7).

## Navigation

| Action | Key |
|--------|-----|
| Move down | `j` or `↓` |
| Move up | `k` or `↑` |
| Open file / enter directory | `Enter`, `l`, or `→` |
| Go up one directory | `h`, `Backspace`, or `←` |
| Search by name | `/` |
| Toggle sort order | `s` |
| Refresh listing | `r` |
| Close | `Escape` |

Files open in the focused terminal pane. Directories are entered in place — the overlay stays open until you press `Escape`.

## Search

Press `/` to enter search mode. Type to filter the current directory listing by filename. Press `Escape` to exit search and return to normal navigation, or `Enter` to open the highlighted match.

## Sort Order

Press `s` to toggle between name-ascending and name-descending sort. The sort persists for the current session.

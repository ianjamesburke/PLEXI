---
title: File Explorer
description: Browse your filesystem without leaving Plexi.
verified_version: "0.0.689"
order: 8
---

The file explorer is a built-in overlay that lets you browse and open files from within any Plexi context. Open it with `⌘E` from a terminal pane — no shell command required.

The layout adapts to the pane width. Narrow splits use a compact list, medium and wide panes switch to a details table, and wide panes can show an explicit inspector with preview metadata for the selected item. Press `Space` for Quick Look in any width.

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
| Toggle recent/name sort | `s` |
| Toggle inspector | `i` |
| Quick Look selected item | `Space` |
| Refresh listing | `r` |
| Close | `Escape` |

Files open in the focused terminal pane. Directories are entered in place — the overlay stays open until you press `Escape`.

## Search

Press `/` to enter search mode. Type to filter the current directory listing by filename. Press `Escape` to exit search and return to normal navigation, or `Enter` to open the highlighted match.

## Details Columns

Medium and wide panes show a details table. Click a column header to sort by that metadata, click the same header again to reverse direction, drag the header sideways to reorder columns, and drag the right edge of a header to resize it.

The toolbar can show or hide metadata columns for kind, size, modified time, creation time, extension, permissions, and tags. The folders-on-top toggle keeps directories grouped above files. Column widths, visibility, sort order, and the folders-on-top setting persist with the File Explorer pane state.

Press `s` to toggle quickly between recent-first and name sort.

## Inspector And Quick Look

Press `i` to toggle the inspector when the pane is wide enough. Drag the divider to resize it. The inspector shows image previews, text previews for common plain-text and source files, folder counts, and generic metadata.

Press `Space` to open Quick Look for the selected item without changing directory or opening a separate app. Press `Space` or `Escape` to dismiss it. Audio and video files still open through the existing Plexi media-player handoff when activated with `Enter`.

# Plexi File Explorer Overhaul PRD

Status: queued behind the Host UI Kit rework.
Last updated: 2026-06-10.

This PRD defines the File Explorer rework. It should not start until the Host UI Kit work in `docs/prm/host-ui-kit.md` has landed far enough that File Explorer can consume shared row, table, button, text field, hint bar, and modal primitives instead of hand-painting its own chrome.

## Purpose

Make File Explorer feel like a first-class Plexi surface, not a proof-of-concept list.

The current browser is useful: it opens from a terminal, follows the linked CWD, has icons, opens media in Plexi players, supports search, and can show basic previews. The layout is the weak part. Rows are fixed-height cards, every item carries too much metadata, and the preview/detail pane appears only after a hard width threshold. On narrow panes, useful information gets pushed into a right-side view the user cannot reach.

The rework should make File Explorer size-aware, dense when space is tight, rich when space exists, and complete enough that users do not need to leave Plexi for normal file work.

## Sequencing

This work comes after the Host UI Kit rework.

Required Host UI Kit pieces:

- `ListRow` or equivalent compact selectable row primitive.
- `DataTable` or column-header primitive for sortable, resizable details view.
- `ModalShell` for Quick Look, destructive confirmations, and file-operation prompts.
- `TextField` for search, rename, and path entry.
- `Button` and icon-button primitives for toolbar and inspector actions.
- `HintBar` for keyboard help.

Do not build File Explorer-specific clones of these primitives. If File Explorer needs a new host widget, add it to the Host UI Kit first, then consume it here.

## Current Truth

- Built-in File Explorer lives in `src/file_browser/mod.rs`.
- Entry data is defined in `src/file_browser/helpers.rs`.
- Icons live in `src/file_browser/icons.rs`.
- `ROW_HEIGHT` is fixed at `58.0`.
- The preview/sidebar breakpoint is fixed at `920.0`.
- Rows are hand-painted two-line cards with icon, name, type/size, and modified time.
- The model tracks only name, path, directory flag, image flag, size, and modified time.
- Sort mode is limited to name and recently touched.
- Search is current-directory fuzzy name filtering.
- File activation routes video/audio to Plexi media apps and falls back to the system opener for other files.
- Website docs describe the current Cmd+E, j/k, Enter, search, sort, refresh, and Escape behavior in `website/src/content/docs/file-explorer.md`.

## Product Model

File Explorer should cover the same core jobs as Finder, adapted for Plexi:

- Browse directories quickly.
- See only the information that fits.
- Sort and group files by useful attributes.
- Search locally and recursively.
- Preview files without opening a separate app.
- Perform common file operations safely.
- Stay linked to the terminal and context model.
- Expose selected paths to agents and host commands.

Finder is the reference for expected file-manager behavior: list/icon/column/gallery views, configurable columns, preview pane, Quick Look, path and status bars, tags, search criteria, and keyboard navigation. Windows Explorer reinforces the same details-view lesson: users can choose columns, resize them, and reorder them.

## Design Principles

- Density first. A file list is a working surface, not a card feed.
- Metadata is contextual. Show what fits, and let users choose columns.
- Preview is optional. It must be useful without trapping information off-screen.
- Narrow panes are real. File Explorer must work inside a split, not only in a wide pane.
- Keyboard and mouse parity. Every common action should be reachable from either path.
- Terminal linkage is a feature. Directory navigation should keep syncing with the linked terminal where appropriate.
- Destructive actions need confirmation and logging.

## Target Layouts

### Compact List

Used in narrow panes.

- Row height around 28-32 px.
- Small icon, single-line filename, optional one trailing value.
- No bordered card per row.
- Metadata hidden unless there is room.
- Selected row remains clear.

### Details Table

Used in medium and wide panes.

- Sortable column headers.
- Resizable columns.
- Hide/show/reorder columns.
- Folders-on-top option.
- Columns include name, kind, size, modified, created, extension, permissions, and tags where supported.

### Inspector

Used only when space exists or when explicitly toggled.

- Resizable side panel on wide panes.
- Bottom drawer or modal on narrow panes.
- Shows preview, metadata, quick actions, path, and selected-count summary.

### Quick Look

Opened with Space.

- Large preview without changing directory.
- Supports images first, then text, PDFs, audio/video handoff, and generic metadata.
- Multiple selection shows a browsable stack or summary.

### View Modes

Not all modes need to ship at once, but the model should allow them:

- List.
- Details.
- Icon grid.
- Column browser.
- Gallery.

The first implementation should ship Compact List plus Details Table. Column, icon, and gallery views can follow.

## Functional Scope

### Navigation

- Back, forward, and up history.
- Breadcrumb/path bar.
- Go to folder by path.
- Open in current pane.
- Open in new pane.
- Reveal in system Finder.
- Copy path.
- Copy shell-escaped path.
- Toggle hidden files.

### Sorting And Grouping

- Sort by name, kind, size, modified, created, extension, and tags.
- Ascending and descending.
- Folders on top.
- Group by kind, date bucket, size bucket, and tag.
- Persist per-folder view preferences where useful.

### Selection

- Single select.
- Multi-select.
- Shift range select.
- Cmd toggle select.
- Select all.
- Selection summary in status bar.

### File Operations

- Rename.
- New folder.
- Copy.
- Cut/move.
- Paste.
- Duplicate.
- Move to trash.
- Open with default app.
- Open with Plexi app.
- Show info.

All destructive operations should go through host-owned confirmation UI and write an info-level log line.

### Search

- Current-folder fuzzy search stays fast.
- Recursive search.
- Criteria search: name, kind, extension, date, size, hidden files, and tags.
- Search scope toggle: current folder vs context root.
- Saved search can wait.

### Preview And Metadata

- Image thumbnail and dimensions.
- Text preview for plain text, Markdown, JSON, TOML, YAML, logs, and source files.
- Folder counts.
- Size and modified/created/accessed times.
- Kind/type.
- Extension.
- Permissions.
- Tags when supported.
- Git status as a later Plexi-specific column.

### Plexi-Specific Features

- Keep terminal CWD sync.
- Send selected path(s) to linked terminal.
- Open selected folder as a context.
- Open selected file in a Plexi app when a matching app exists.
- Expose current selection to host commands and agents.
- Keep actions capability-aware when File Explorer calls into host APIs.

## Non-Goals

- Do not replace macOS Finder.
- Do not build a separate file indexer in the first pass.
- Do not add cloud file-provider integrations in this PRD.
- Do not add a second design system for File Explorer.
- Do not rewrite media players as part of this work.
- Do not maintain `apps/dev/` examples for this lane.

## Issue Bundle

These are the implementation issues this PRD points to. File them with `/create-issue` when this PRD is ready to enter the queue, and link the resulting GitHub issue numbers back here.

1. File Explorer: compact adaptive list/details layout

   Build the responsive shell, compact row layout, details table breakpoint, toolbar, path/status bars, and row-density model.

2. File Explorer: column model, sorting, resizing, and persistence

   Add a real column model, sort descriptors, visible-column settings, resizing/reordering, folders-on-top, and per-folder or per-session persistence.

3. File Explorer: inspector and Quick Look overlay

   Replace the fixed right preview threshold with a toggled/resizable inspector and a Space-driven Quick Look modal.

4. File Explorer: multi-select and safe file operations

   Add multi-select plus rename, new folder, copy/cut/paste, duplicate, move to trash, reveal, open with default, and confirmation/logging for destructive operations.

5. File Explorer: recursive search and filters

   Expand current fuzzy filtering into scoped recursive search with criteria for name, kind, extension, date, size, hidden files, and tags.

6. File Explorer: icon, column, and gallery views

   Add richer view modes after compact list and details table are stable.

7. File Explorer: Plexi-native actions and agent selection state

   Expose selected paths to linked terminals, host commands, Plexi apps, and agents without bypassing capability or context boundaries.

## Implementation Map

- `src/file_browser/mod.rs` - split state, layout, input handling, row rendering, preview behavior, and file-operation dispatch into smaller units.
- `src/file_browser/helpers.rs` - expand `Entry`, `SortMode`, formatting helpers, and metadata extraction.
- `src/file_browser/icons.rs` - keep icon classification, but adapt icon sizes to compact rows and grid views.
- `src/ui/widgets.rs` - consume Host UI Kit primitives; add missing shared widgets there before using them in File Explorer.
- `src/ui/style.rs` - add row/table/inspector tokens only when shared by more than one host surface.
- `src/app/app_trait.rs` and pane dispatch paths - use existing app command patterns for open-in-pane, CWD sync, and host actions.
- `website/src/content/docs/file-explorer.md` - update docs after behavior lands.

Do not touch:

- `apps/dev/` examples unless a temporary proof-of-concept is explicitly needed.
- Media player internals unless the File Explorer handoff contract changes.
- App marketplace or PGAP protocol code unless a selected-path host API is deliberately added.

## Done When

- File Explorer is usable in a narrow split without losing core information.
- Wide panes show a details table with sortable, configurable columns.
- Preview and metadata are available without a fixed unreachable right panel.
- Multi-select and common file operations work with confirmations where needed.
- Search handles current-folder and recursive use cases.
- Keyboard navigation remains fast.
- Terminal CWD sync still works.
- `cargo build` passes.
- Focused `cargo test --bin plexi file_browser` coverage passes, plus any new HostHarness tests for host actions.

## References

- `docs/prm/host-ui-kit.md`
- `NORTH_STAR.md`
- `website/src/content/docs/file-explorer.md`
- Apple Finder user guide: view modes, columns, preview, Quick Look, search criteria, tags, path/status bars, and Finder shortcuts.
- Microsoft Windows Explorer column settings: visible columns, width, order, and default details columns.
